use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing::{debug, warn};

use crate::types::{Asset, Direction, PolymarketSignal};

// ── Polymarket CLOB API response shapes ──────────────────────────────────────

/// Raw market from GET /markets
#[derive(Debug, Deserialize)]
struct RawMarket {
    condition_id: String,
    question: String,
    active: bool,
    closed: bool,
    tokens: Vec<RawToken>,
    end_date_iso: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawToken {
    token_id: String,
    outcome: String,  // "Yes" or "No"
    price: String,    // decimal string e.g. "0.5412"
}

/// Raw orderbook from GET /book?token_id=...
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

/// Parse the market question to identify which asset + direction + timeframe it covers.
/// Polymarket uses questions like:
///   "Will BTC be higher in 15 minutes?"
///   "ETH up in the next 1 hour?"
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

    // Direction: "higher", "up", "above" → Up; "lower", "down", "below" → Down
    let direction = if q.contains("higher")
        || q.contains(" up ")
        || q.contains("above")
        || q.contains("increase")
    {
        Direction::Up
    } else if q.contains("lower")
        || q.contains("down")
        || q.contains("below")
        || q.contains("decrease")
    {
        Direction::Down
    } else {
        // Default ambiguous markets to "Up" — caller can filter
        Direction::Up
    };

    Some((asset, direction))
}

// ── Scanner ───────────────────────────────────────────────────────────────────

pub struct PolymarketScanner {
    client: Client,
    base_url: String,
}

impl PolymarketScanner {
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

    /// Fetch all active short-term crypto prediction markets
    pub async fn fetch_signals(&self) -> Result<Vec<PolymarketSignal>> {
        let markets = self.fetch_active_markets().await?;
        let mut signals = Vec::new();

        for market in markets {
            match self.market_to_signal(market).await {
                Ok(Some(signal)) => signals.push(signal),
                Ok(None) => {} // not a short-term crypto market, skip
                Err(e) => warn!("Failed to process market: {}", e),
            }
        }

        debug!("Fetched {} Polymarket signals", signals.len());
        Ok(signals)
    }

    async fn fetch_active_markets(&self) -> Result<Vec<RawMarket>> {
        // Polymarket CLOB: GET /markets?active=true&closed=false&tag_slug=crypto
        let url = format!("{}/markets?active=true&closed=false&tag_slug=crypto", self.base_url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("GET /markets failed")?;

        // The response is { data: [...markets...], next_cursor: "..." }
        #[derive(Deserialize)]
        struct MarketsResponse {
            data: Vec<RawMarket>,
        }

        let body: MarketsResponse = resp
            .json()
            .await
            .context("Failed to parse /markets response")?;

        Ok(body.data)
    }

    async fn market_to_signal(&self, market: RawMarket) -> Result<Option<PolymarketSignal>> {
        // Skip inactive / closed
        if !market.active || market.closed {
            return Ok(None);
        }

        // Must match a known asset + direction
        let (asset, direction) = match parse_market_question(&market.question) {
            Some(d) => d,
            None => return Ok(None),
        };

        // Must have a resolution time
        let resolves_at = match &market.end_date_iso {
            Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
                .context("bad end_date_iso")?
                .with_timezone(&Utc),
            None => return Ok(None),
        };

        // We only care about markets resolving within the next 2 hours
        let mins_to_resolution = (resolves_at - Utc::now()).num_minutes();
        if mins_to_resolution <= 0 || mins_to_resolution > 120 {
            return Ok(None);
        }

        // Find YES token
        let yes_token = market
            .tokens
            .iter()
            .find(|t| t.outcome.to_lowercase() == "yes")
            .ok_or_else(|| anyhow::anyhow!("no YES token in market {}", market.condition_id))?;

        let yes_price = Decimal::from_str(&yes_token.price)
            .context("bad YES price string")?;

        // Fetch orderbook for this YES token
        let (bid, ask, liquidity) = self
            .fetch_orderbook_top(&yes_token.token_id)
            .await
            .unwrap_or_else(|_| (yes_price, yes_price, dec!(0)));

        let mid = (bid + ask) / dec!(2);

        Ok(Some(PolymarketSignal {
            asset,
            direction,
            resolves_at,
            yes_bid: bid,
            yes_ask: ask,
            yes_mid: mid,
            yes_liquidity: liquidity,
            condition_id: market.condition_id,
            yes_token_id: yes_token.token_id.clone(),
            captured_at: Utc::now(),
        }))
    }

    /// Returns (best_bid, best_ask, total_bid_liquidity_usdc) for a YES token
    async fn fetch_orderbook_top(&self, token_id: &str) -> Result<(Decimal, Decimal, Decimal)> {
        let url = format!("{}/book?token_id={}", self.base_url, token_id);

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
        // bid size is in outcome tokens; multiply by price to get USDC value
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
    /// From Polymarket's dynamic fee model (Jan 2026):
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
        let q = "Will BTC be higher in 15 minutes?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::BTC, Direction::Up)));
    }

    #[test]
    fn test_parse_question_eth_down() {
        let q = "ETH price lower in 1 hour?";
        let result = parse_market_question(q);
        assert_eq!(result, Some((Asset::ETH, Direction::Down)));
    }

    #[test]
    fn test_parse_question_unknown() {
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
