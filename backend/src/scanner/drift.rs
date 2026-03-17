use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::str::FromStr;
use tracing::debug;

use crate::types::{Asset, DriftSignal};

// ── Drift DLOB API response shapes ──────────────────────────────────────────
// DLOB API (dlob.drift.trade) is public and provides real-time market data.
// Data API (data.api.drift.trade) provides historical funding rates.

/// DLOB L2 orderbook response (also includes market metadata)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DlobL2Response {
    market_name: String,
    market_index: u16,
    /// Mark price scaled by 1e6
    mark_price: String,
    /// Oracle price (raw integer scaled by 1e6)
    oracle: i64,
}

/// Funding rate record from data API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FundingRateRecord {
    /// Funding rate scaled by 1e9
    funding_rate: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FundingRatesResponse {
    funding_rates: Vec<FundingRateRecord>,
}

// ── Known markets ───────────────────────────────────────────────────────────

const MARKETS: &[(&str, Asset)] = &[
    ("SOL-PERP", Asset::SOL),
    ("BTC-PERP", Asset::BTC),
    ("ETH-PERP", Asset::ETH),
];

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
    Ok(raw_int / DRIFT_FUNDING_PRECISION)
}

// ── Scanner ───────────────────────────────────────────────────────────────────

const DLOB_API: &str = "https://dlob.drift.trade";
const DATA_API: &str = "https://data.api.drift.trade";

pub struct DriftScanner {
    client: Client,
    /// Kept for potential gateway use, but scanning uses public APIs
    _base_url: String,
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
            _base_url: base_url.to_string(),
        }
    }

    /// Fetch all supported assets at once via DLOB public API
    pub async fn fetch_all_signals(&self) -> Result<Vec<DriftSignal>> {
        let mut signals = Vec::new();

        for (market_name, asset) in MARKETS {
            match self.fetch_market_signal(market_name, asset).await {
                Ok(signal) => signals.push(signal),
                Err(e) => {
                    debug!("Failed to fetch {}: {}", market_name, e);
                }
            }
        }

        debug!("Fetched {} Drift signals", signals.len());
        Ok(signals)
    }

    async fn fetch_market_signal(&self, market_name: &str, asset: &Asset) -> Result<DriftSignal> {
        // Fetch L2 orderbook (includes mark price, oracle, market index)
        let l2_url = format!("{}/l2?marketName={}&depth=1", DLOB_API, market_name);
        let l2: DlobL2Response = self
            .client
            .get(&l2_url)
            .send()
            .await
            .context("GET /l2 failed")?
            .json()
            .await
            .context("Failed to parse /l2 response")?;

        let mark_price = parse_drift_price(&l2.mark_price)?;
        let oracle_price = Decimal::from(l2.oracle) / DRIFT_PRICE_PRECISION;

        // Mark premium = (mark - oracle) / oracle
        let mark_premium = if oracle_price > dec!(0) {
            (mark_price - oracle_price) / oracle_price
        } else {
            dec!(0)
        };

        // Fetch latest funding rate from data API
        let funding_rate_1h = self
            .fetch_latest_funding(l2.market_index)
            .await
            .unwrap_or(dec!(0));

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
            market_index: l2.market_index,
            captured_at: Utc::now(),
        })
    }

    async fn fetch_latest_funding(&self, market_index: u16) -> Result<Decimal> {
        let url = format!("{}/fundingRates?marketIndex={}", DATA_API, market_index);

        let resp: FundingRatesResponse = self
            .client
            .get(&url)
            .send()
            .await
            .context("GET /fundingRates failed")?
            .json()
            .await
            .context("Failed to parse /fundingRates response")?;

        // Latest funding rate is the last entry
        let latest = resp
            .funding_rates
            .last()
            .ok_or_else(|| anyhow::anyhow!("no funding rates returned"))?;

        parse_drift_funding(&latest.funding_rate)
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
