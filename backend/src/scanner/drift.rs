use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::debug;

use crate::types::{Asset, DriftSignal};

// ── Drift API response shapes ─────────────────────────────────────────────────
// Drift's public REST API: https://mainnet-beta.api.drift.trade/v2/

/// Drift market info from GET /v2/perpMarkets
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPerpMarket {
    market_index: u16,
    symbol: String,           // e.g. "BTC-PERP"
    mark_price: String,       // scaled by 1e6
    oracle_price: String,     // scaled by 1e6
    last_funding_rate: String, // scaled by 1e9, per hour
}

/// Drift funding rate from GET /v2/fundingRates/{marketIndex}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFundingRate {
    funding_rate: String, // scaled by 1e9
    ts: String,
}

// ── Drift market index map ────────────────────────────────────────────────────

/// Known Drift perpetual market indices (stable, defined in Drift protocol)
fn drift_market_index(asset: &Asset) -> u16 {
    match asset {
        Asset::BTC => 1,
        Asset::ETH => 2,
        Asset::SOL => 0, // SOL-PERP is market 0
    }
}

fn symbol_to_asset(symbol: &str) -> Option<Asset> {
    match symbol {
        "BTC-PERP" => Some(Asset::BTC),
        "ETH-PERP" => Some(Asset::ETH),
        "SOL-PERP" => Some(Asset::SOL),
        _ => None,
    }
}

// ── Drift price scaling ───────────────────────────────────────────────────────

/// Drift encodes prices as integers scaled by PRICE_PRECISION = 10^6
const DRIFT_PRICE_PRECISION: Decimal = dec!(1_000_000);

/// Drift encodes funding rates as integers scaled by FUNDING_PRECISION = 10^9
const DRIFT_FUNDING_PRECISION: Decimal = dec!(1_000_000_000);

fn parse_drift_price(raw: &str) -> Result<Decimal> {
    let raw_int = Decimal::from_str(raw).context("bad price string")?;
    Ok(raw_int / DRIFT_PRICE_PRECISION)
}

fn parse_drift_funding(raw: &str) -> Result<Decimal> {
    let raw_int = Decimal::from_str(raw).context("bad funding string")?;
    // Result is in units of "per oracle price", divide by PRECISION to get a ratio
    // Then that ratio applied every hour → this is the hourly funding rate
    Ok(raw_int / DRIFT_FUNDING_PRECISION)
}

// ── Scanner ───────────────────────────────────────────────────────────────────

pub struct DriftScanner {
    client: Client,
    base_url: String,
}

impl DriftScanner {
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("SolArb-Agent/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.to_string(),
        }
    }

    /// Fetch DriftSignal for a specific asset
    pub async fn fetch_signal(&self, asset: &Asset) -> Result<DriftSignal> {
        let market_index = drift_market_index(asset);

        // Fetch all perp markets in one call (more efficient than one per asset)
        let markets = self.fetch_perp_markets().await?;

        let market = markets
            .get(&market_index)
            .ok_or_else(|| anyhow::anyhow!("Drift market {} not found", market_index))?;

        let mark_price = parse_drift_price(&market.mark_price)?;
        let oracle_price = parse_drift_price(&market.oracle_price)?;
        let funding_rate_1h = parse_drift_funding(&market.last_funding_rate)?;

        // Mark premium = (mark - oracle) / oracle
        let mark_premium = if oracle_price > dec!(0) {
            (mark_price - oracle_price) / oracle_price
        } else {
            dec!(0)
        };

        debug!(
            "Drift {}: mark={:.2} oracle={:.2} premium={:.4}% funding={:.6}%/hr",
            asset,
            mark_price,
            oracle_price,
            mark_premium * dec!(100),
            funding_rate_1h * dec!(100),
        );

        Ok(DriftSignal {
            asset: asset.clone(),
            funding_rate_1h,
            mark_price,
            oracle_price,
            mark_premium,
            market_index,
            captured_at: Utc::now(),
        })
    }

    /// Fetch all supported assets at once
    pub async fn fetch_all_signals(&self) -> Result<Vec<DriftSignal>> {
        let markets = self.fetch_perp_markets().await?;
        let mut signals = Vec::new();

        for (_, market) in &markets {
            let asset = match symbol_to_asset(&market.symbol) {
                Some(a) => a,
                None => continue,
            };

            let mark_price = parse_drift_price(&market.mark_price)
                .unwrap_or(dec!(0));
            let oracle_price = parse_drift_price(&market.oracle_price)
                .unwrap_or(dec!(0));
            let funding_rate_1h = parse_drift_funding(&market.last_funding_rate)
                .unwrap_or(dec!(0));

            let mark_premium = if oracle_price > dec!(0) {
                (mark_price - oracle_price) / oracle_price
            } else {
                dec!(0)
            };

            signals.push(DriftSignal {
                asset,
                funding_rate_1h,
                mark_price,
                oracle_price,
                mark_premium,
                market_index: market.market_index,
                captured_at: Utc::now(),
            });
        }

        debug!("Fetched {} Drift signals", signals.len());
        Ok(signals)
    }

    async fn fetch_perp_markets(&self) -> Result<HashMap<u16, RawPerpMarket>> {
        let url = format!("{}/v2/perpMarkets", self.base_url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("GET /v2/perpMarkets failed")?;

        #[derive(Deserialize)]
        struct PerpMarketsResponse {
            success: bool,
            data: Vec<RawPerpMarket>,
        }

        let body: PerpMarketsResponse = resp
            .json()
            .await
            .context("Failed to parse /v2/perpMarkets")?;

        if !body.success {
            anyhow::bail!("Drift API returned success=false");
        }

        let map = body
            .data
            .into_iter()
            .map(|m| (m.market_index, m))
            .collect();

        Ok(map)
    }

    /// Estimate Drift trading fee.
    /// Drift uses a tiered fee structure based on account tier.
    /// For a fresh account: taker ~0.1% (10bps), maker rebate ~0.02%
    pub fn estimate_taker_fee() -> Decimal {
        dec!(0.001) // 0.1%
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_drift_price() {
        // 65000 USD * 1e6 = 65_000_000_000
        let raw = "65000000000";
        let price = parse_drift_price(raw).unwrap();
        assert_eq!(price, dec!(65000));
    }

    #[test]
    fn test_parse_drift_funding() {
        // 0.01%/hr = 0.0001 → 0.0001 * 1e9 = 100_000
        let raw = "100000";
        let rate = parse_drift_funding(raw).unwrap();
        assert!((rate - dec!(0.0001)).abs() < dec!(0.00001));
    }

    #[test]
    fn test_implied_probability_neutral() {
        let signal = DriftSignal {
            asset: Asset::BTC,
            funding_rate_1h: dec!(0),
            mark_price: dec!(65000),
            oracle_price: dec!(65000),
            mark_premium: dec!(0),
            market_index: 1,
            captured_at: Utc::now(),
        };
        // With zero premium and zero funding, probability should be ~50%
        let prob = signal.implied_up_probability(1);
        assert!((prob - dec!(0.5)).abs() < dec!(0.01));
    }

    #[test]
    fn test_implied_probability_bullish_funding() {
        let signal = DriftSignal {
            asset: Asset::BTC,
            funding_rate_1h: dec!(0.0002), // 0.02%/hr — distinctly bullish
            mark_price: dec!(65100),
            oracle_price: dec!(65000),
            mark_premium: dec!(0.00154), // ~0.15% premium
            market_index: 1,
            captured_at: Utc::now(),
        };
        let prob = signal.implied_up_probability(1);
        // Should be above 50% due to bullish signals
        assert!(prob > dec!(0.5), "prob was {}", prob);
        assert!(prob < dec!(0.75), "prob should be moderate, was {}", prob);
    }
}
