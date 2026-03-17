use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{debug, warn};

use crate::types::{Asset, Direction, PolymarketSignal};

// ── Gamma API response shape ─────────────────────────────────────────────────
// Gamma API (gamma-api.polymarket.com) provides proper market search/filtering.
// CLOB API (clob.polymarket.com) is used only for orderbook depth.

/// Market from Gamma API: GET /markets?active=true&closed=false&...
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GammaMarket {
    /// Polymarket condition ID
    condition_id: String,
    /// Market question text
    question: String,
    /// Whether the market is active
    active: bool,
    /// Whether the market is closed
    closed: bool,
    /// ISO end date (e.g. "2026-03-17T16:00:00Z")
    end_date: Option<String>,
    /// JSON-encoded outcome names, e.g. "[\"Yes\", \"No\"]"
    outcomes: Option<String>,
    /// JSON-encoded outcome prices, e.g. "[\"0.45\", \"0.55\"]"
    outcome_prices: Option<String>,
    /// JSON-encoded CLOB token IDs, e.g. "[\"123...\", \"456...\"]"
    clob_token_ids: Option<String>,
    /// USDC liquidity
    liquidity_num: Option<f64>,
    /// Whether the market accepts orders
    accepting_orders: Option<bool>,
}

/// CLOB API orderbook
#[derive(Debug, Deserialize)]
struct RawBook {
    bids: Vec<RawLevel>,
    asks: Vec<RawLevel>,
}

#[derive(Debug, Deserialize)]
struct RawLevel {
    price: String,
    size: String,
}

// ── Market discovery ──────────────────────────────────────────────────────────

/// Parse the market question to identify which asset + direction it covers.
/// Polymarket uses questions like:
///   "Will the price of Bitcoin be above $78,000 on March 17?"
///   "Will Bitcoin reach $90,000 in March?"
///   "ETH above $3000?"
fn parse_market_question(question: &str) -> Option<(Asset, Direction)> {
    let q = question.to_lowercase();

    let asset = if q.contains("btc") || q.contains("bitcoin") {
        Asset::BTC
    } else if q.contains("eth") || q.contains("ethereum") {
        Asset::ETH
    } else if q.contains("sol") || q.contains("solana") {
        Asset::SOL
    } else {
        return None;
    };

    // Must be a price/market prediction, not a person/event question
    let is_price_market = q.contains("price")
        || q.contains("above")
        || q.contains("below")
        || q.contains("reach")
        || q.contains("higher")
        || q.contains("lower")
        || q.contains(" up ")
        || q.contains("down")
        || q.contains("dip")
        || q.contains("hit $")
        || q.contains("$");

    if !is_price_market {
        return None;
    }

    // Direction: "higher", "up", "above", "reach" → Up; "lower", "down", "below", "dip" → Down
    let direction = if q.contains("lower")
        || q.contains("down")
        || q.contains("below")
        || q.contains("decrease")
        || q.contains("dip")
    {
        Direction::Down
    } else {
        // Default for price markets ("above", "reach", "higher") is Up
        Direction::Up
    };

    Some((asset, direction))
}

// ── Scanner ───────────────────────────────────────────────────────────────────

const GAMMA_API: &str = "https://gamma-api.polymarket.com";

pub struct PolymarketScanner {
    client: Client,
    clob_url: String,
}

impl PolymarketScanner {
    pub fn new(clob_base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("SolArb-Agent/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            clob_url: clob_base_url.to_string(),
        }
    }

    /// Fetch all active short-term crypto prediction markets
    pub async fn fetch_signals(&self) -> Result<Vec<PolymarketSignal>> {
        let markets = self.fetch_active_markets().await?;
        let mut signals = Vec::new();

        for market in markets {
            match self.market_to_signal(market).await {
                Ok(Some(signal)) => signals.push(signal),
                Ok(None) => {} // not a short-term crypto price market, skip
                Err(e) => warn!("Failed to process market: {}", e),
            }
        }

        debug!("Fetched {} Polymarket signals", signals.len());
        Ok(signals)
    }

    async fn fetch_active_markets(&self) -> Result<Vec<GammaMarket>> {
        // Gamma API returns a direct JSON array (no wrapper)
        let url = format!(
            "{}/markets?active=true&closed=false&order=volume24hr&ascending=false&limit=100",
            GAMMA_API
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("GET /markets (Gamma) failed")?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("Gamma API returned {}", status);
        }

        let text = resp.text().await.context("Failed to read Gamma response body")?;

        let markets: Vec<GammaMarket> = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                debug!("Gamma parse error: {} | body prefix: {}", e, &text[..text.len().min(300)]);
                return Err(anyhow::anyhow!("Failed to parse Gamma API: {}", e));
            }
        };

        debug!("Gamma API returned {} raw markets", markets.len());
        Ok(markets)
    }

    async fn market_to_signal(&self, market: GammaMarket) -> Result<Option<PolymarketSignal>> {
        // Skip inactive / closed / not accepting orders
        if !market.active || market.closed {
            return Ok(None);
        }

        // Must match a known asset + direction (filters out non-crypto markets)
        let (asset, direction) = match parse_market_question(&market.question) {
            Some(d) => d,
            None => return Ok(None),
        };

        // Must have a resolution time
        let end_date_str = match &market.end_date {
            Some(ts) if !ts.is_empty() => ts.clone(),
            _ => return Ok(None),
        };

        let resolves_at = chrono::DateTime::parse_from_rfc3339(&end_date_str)
            .or_else(|_| {
                // Gamma sometimes returns date-only like "2026-03-17"
                chrono::DateTime::parse_from_rfc3339(&format!("{}T23:59:59Z", end_date_str))
            })
            .context("bad end_date")?
            .with_timezone(&Utc);

        // Include markets resolving within the next 30 days
        let mins_to_resolution = (resolves_at - Utc::now()).num_minutes();
        if mins_to_resolution <= 0 || mins_to_resolution > 43200 {
            return Ok(None);
        }

        // Parse outcome prices from JSON-encoded string
        let prices: Vec<String> = match &market.outcome_prices {
            Some(s) => serde_json::from_str(s).unwrap_or_default(),
            None => return Ok(None),
        };

        // Parse outcomes to find YES index
        let outcomes: Vec<String> = match &market.outcomes {
            Some(s) => serde_json::from_str(s).unwrap_or_default(),
            None => return Ok(None),
        };

        let yes_idx = match outcomes.iter().position(|o| o.to_lowercase() == "yes") {
            Some(idx) => idx,
            None => return Ok(None), // multi-outcome market (not Yes/No), skip
        };

        let yes_price = prices
            .get(yes_idx)
            .and_then(|p| Decimal::from_str(p).ok())
            .unwrap_or(dec!(0.5));

        // Parse CLOB token IDs
        let token_ids: Vec<String> = match &market.clob_token_ids {
            Some(s) => serde_json::from_str(s).unwrap_or_default(),
            None => return Ok(None),
        };

        let yes_token_id = match token_ids.get(yes_idx) {
            Some(id) => id.clone(),
            None => return Ok(None),
        };

        // Fetch orderbook from CLOB API for this YES token
        let (bid, ask, liquidity) = self
            .fetch_orderbook_top(&yes_token_id)
            .await
            .unwrap_or_else(|_| (yes_price, yes_price, dec!(0)));

        // Use orderbook mid if we have both sides, otherwise use Gamma price
        let mid = if bid > dec!(0) && ask < dec!(1) && ask > bid {
            (bid + ask) / dec!(2)
        } else {
            yes_price
        };

        // Also use Gamma liquidity if CLOB returned none
        let total_liquidity = if liquidity > dec!(0) {
            liquidity
        } else {
            Decimal::try_from(market.liquidity_num.unwrap_or(0.0)).unwrap_or(dec!(0))
        };

        debug!(
            "Polymarket {} {}: YES={:.3} bid={:.3} ask={:.3} mid={:.3} liq=${:.0} T-{}min",
            asset, direction, yes_price, bid, ask, mid, total_liquidity, mins_to_resolution
        );

        Ok(Some(PolymarketSignal {
            asset,
            direction,
            resolves_at,
            yes_bid: bid,
            yes_ask: ask,
            yes_mid: mid,
            yes_liquidity: total_liquidity,
            condition_id: market.condition_id,
            yes_token_id,
            captured_at: Utc::now(),
        }))
    }

    /// Returns (best_bid, best_ask, total_bid_liquidity_usdc) for a YES token
    /// Uses CLOB API for real-time orderbook depth.
    async fn fetch_orderbook_top(&self, token_id: &str) -> Result<(Decimal, Decimal, Decimal)> {
        let url = format!("{}/book?token_id={}", self.clob_url, token_id);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("GET /book failed")?;

        let book: RawBook = resp.json().await.context("Failed to parse /book")?;

        // Best bid = highest bid price
        let best_bid = book
            .bids
            .iter()
            .filter_map(|l| Decimal::from_str(&l.price).ok())
            .fold(dec!(0), Decimal::max);

        // Best ask = lowest ask price
        let best_ask = book
            .asks
            .iter()
            .filter_map(|l| Decimal::from_str(&l.price).ok())
            .fold(dec!(1), Decimal::min);

        // Sum bid-side liquidity (in USDC)
        let bid_liquidity: Decimal = book
            .bids
            .iter()
            .filter_map(|l| {
                let price = Decimal::from_str(&l.price).ok()?;
                let size = Decimal::from_str(&l.size).ok()?;
                Some(price * size)
            })
            .sum();

        Ok((best_bid, best_ask, bid_liquidity))
    }

    /// Estimate Polymarket taker fee for a given YES price.
    ///
    /// From Polymarket's dynamic fee model:
    ///   fee ≈ 3.15% when price ≈ $0.50 (most uncertain / highest spread)
    ///   fee approaches 0 as price → 0 or → 1 (one-sided certainty)
    ///
    /// Formula: fee = k * p * (1 - p)  where k ≈ 0.126 gives 3.15% at p=0.5
    pub fn estimate_taker_fee(yes_price: Decimal) -> Decimal {
        let k = dec!(0.126);
        let p = yes_price.max(dec!(0.01)).min(dec!(0.99));
        k * p * (dec!(1) - p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_question_btc_up() {
        let q = "Will the price of Bitcoin be above $78,000 on March 17?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::BTC, Direction::Up)));
    }

    #[test]
    fn test_parse_question_btc_reach() {
        let q = "Will Bitcoin reach $90,000 in March?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::BTC, Direction::Up)));
    }

    #[test]
    fn test_parse_question_btc_dip() {
        let q = "Will Bitcoin dip to $65,000 in March?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::BTC, Direction::Down)));
    }

    #[test]
    fn test_parse_question_eth_down() {
        let q = "ETH price lower in 1 hour?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::ETH, Direction::Down)));
    }

    #[test]
    fn test_parse_question_non_price() {
        // "BitBoy convicted?" mentions crypto tag but is not a price market
        let q = "BitBoy convicted?";
        let result = parse_market_question(q);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_question_unknown_asset() {
        let q = "Will LINK reach $20?";
        let result = parse_market_question(q);
        assert!(result.is_none());
    }

    #[test]
    fn test_taker_fee_at_50_cents() {
        // Fee should be ~3.15% at p=0.5
        let fee = PolymarketScanner::estimate_taker_fee(dec!(0.5));
        let expected = dec!(0.0315);
        let diff = (fee - expected).abs();
        assert!(diff < dec!(0.001), "fee at 0.5 = {}", fee);
    }

    #[test]
    fn test_taker_fee_low_at_extremes() {
        // Fee should be much lower at extremes
        let fee_low = PolymarketScanner::estimate_taker_fee(dec!(0.1));
        let fee_high = PolymarketScanner::estimate_taker_fee(dec!(0.9));
        assert!(fee_low < dec!(0.012));
        assert!(fee_high < dec!(0.012));
    }
}
