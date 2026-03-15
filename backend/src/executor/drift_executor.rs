use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::types::{Asset, PositionSide};
use crate::wallet::SolWallet;

// ── Drift Gateway REST API types ────────────────────────────────────────────
// Compatible with drift-labs/gateway (https://github.com/drift-labs/gateway)

/// Order request sent to POST /v2/orders
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayOrderRequest {
    market_index: u16,
    market_type: &'static str,
    /// Positive = long, negative = short (in base asset units)
    amount: f64,
    /// "market" or "limit"
    order_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<f64>,
    reduce_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_only: Option<bool>,
}

/// Response from gateway order endpoints
#[derive(Debug, Deserialize)]
struct GatewayOrderResponse {
    tx: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Position from GET /v2/positions
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPosition {
    pub market_index: u16,
    pub market_type: String,
    pub amount: String,
    pub entry_price: Option<String>,
    pub oracle_price: Option<String>,
    pub unrealized_pnl: Option<String>,
}

/// Perp market data from DLOB server
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DlobMarketResponse {
    market_index: Option<u16>,
    mark_price: Option<String>,
    oracle_price: Option<String>,
}

// ── Drift executor ───────────────────────────────────────────────────────────

pub struct DriftExecutor {
    client: Client,
    /// Drift Gateway URL (e.g. http://localhost:8080)
    gateway_url: String,
    /// Drift DLOB server URL for market data (e.g. https://dlob.drift.trade)
    dlob_url: String,
    wallet: Option<Arc<SolWallet>>,
}

impl DriftExecutor {
    pub fn new(wallet: Arc<SolWallet>, gateway_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("SolArb-Agent/0.3")
            .build()
            .expect("failed to build HTTP client");

        // DLOB server for read-only market data
        let dlob_url = if gateway_url.contains("devnet") {
            "https://dlob.drift.trade".to_string()
        } else {
            "https://dlob.drift.trade".to_string()
        };

        Self {
            client,
            gateway_url: gateway_url.to_string(),
            dlob_url,
            wallet: Some(wallet),
        }
    }

    pub fn new_dry() -> Self {
        Self {
            client: Client::new(),
            gateway_url: String::new(),
            dlob_url: String::new(),
            wallet: None,
        }
    }

    /// Check if the gateway is reachable and user account is initialized
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/v2/positions", self.gateway_url);
        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("Gateway health check failed")?;

        Ok(resp.status().is_success())
    }

    /// Open a perpetual position via the Drift Gateway
    pub async fn open_perp_position(
        &self,
        asset: &Asset,
        side: &PositionSide,
        size_usdc: Decimal,
        market_index: u16,
    ) -> Result<String> {
        // Get mark price to calculate base amount from USDC size
        let mark_price = self.get_mark_price(market_index).await
            .unwrap_or_else(|_| {
                warn!("Could not fetch mark price, using size directly as amount");
                dec!(1)
            });

        // Convert USDC size to base asset amount
        let base_amount = if mark_price > dec!(0) {
            size_usdc / mark_price
        } else {
            size_usdc
        };

        // Negative amount for short
        let amount = match side {
            PositionSide::Long => dec_to_f64(base_amount),
            PositionSide::Short => -dec_to_f64(base_amount),
        };

        let side_str = match side {
            PositionSide::Long => "long",
            PositionSide::Short => "short",
        };

        info!(
            "Drift: opening {} {} perp | size=${} | base_amount={:.6} | market_index={}",
            side_str, asset, size_usdc, amount.abs(), market_index
        );

        let order = GatewayOrderRequest {
            market_index,
            market_type: "perp",
            amount,
            order_type: "market",
            price: None,
            reduce_only: false,
            post_only: None,
        };

        let sig = self.submit_order(&order).await?;
        info!("Drift perp opened: {} {} | tx: {}", side_str, asset, sig);
        Ok(sig)
    }

    /// Close a perpetual position via the Drift Gateway
    pub async fn close_perp_position(&self, market_index: u16) -> Result<String> {
        info!("Drift: closing perp position on market_index={}", market_index);

        // Query current position to get the amount to close
        let positions = self.get_positions().await?;
        let position = positions.iter()
            .find(|p| p.market_index == market_index && p.market_type == "perp");

        let close_amount = match position {
            Some(pos) => {
                let amount: f64 = pos.amount.parse().unwrap_or(0.0);
                -amount // Reverse direction to close
            }
            None => {
                warn!("No open position found for market_index={}, sending reduce_only order", market_index);
                0.0
            }
        };

        if close_amount.abs() < f64::EPSILON {
            // No position to close, or zero amount
            info!("No position to close on market_index={}", market_index);
            return Ok("no-position".to_string());
        }

        let order = GatewayOrderRequest {
            market_index,
            market_type: "perp",
            amount: close_amount,
            order_type: "market",
            price: None,
            reduce_only: true,
            post_only: None,
        };

        let sig = self.submit_order(&order).await?;
        info!("Drift perp closed: market_index={} | tx: {}", market_index, sig);
        Ok(sig)
    }

    /// Get current mark price for a perp market from DLOB server
    pub async fn get_mark_price(&self, market_index: u16) -> Result<Decimal> {
        // Try gateway first (more accurate for the user's context)
        let gateway_url = format!("{}/v2/perpMarkets", self.gateway_url);
        if let Ok(resp) = self.client.get(&gateway_url).send().await {
            if resp.status().is_success() {
                if let Ok(markets) = resp.json::<Vec<DlobMarketResponse>>().await {
                    if let Some(market) = markets.iter().find(|m| m.market_index == Some(market_index)) {
                        if let Some(price_str) = &market.mark_price {
                            if let Ok(price) = price_str.parse::<Decimal>() {
                                // Gateway prices may be in PRICE_PRECISION (1e6)
                                let normalized = if price > dec!(1_000_000) {
                                    price / dec!(1_000_000)
                                } else {
                                    price
                                };
                                return Ok(normalized);
                            }
                        }
                    }
                }
            }
        }

        // Fallback to DLOB server
        let dlob_url = format!(
            "{}/l2?marketIndex={}&marketType=perp&depth=1",
            self.dlob_url, market_index
        );

        let resp = self.client
            .get(&dlob_url)
            .send()
            .await
            .context("DLOB GET mark price failed")?;

        if !resp.status().is_success() {
            anyhow::bail!("DLOB mark price request failed: {}", resp.status());
        }

        // Parse the L2 orderbook to extract mid price
        let body: serde_json::Value = resp.json().await
            .context("Failed to parse DLOB response")?;

        let best_bid = body["bids"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b["price"].as_str())
            .and_then(|s| s.parse::<Decimal>().ok());

        let best_ask = body["asks"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b["price"].as_str())
            .and_then(|s| s.parse::<Decimal>().ok());

        match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => {
                let mid = (bid + ask) / dec!(2);
                let normalized = if mid > dec!(1_000_000) {
                    mid / dec!(1_000_000)
                } else {
                    mid
                };
                Ok(normalized)
            }
            (Some(price), None) | (None, Some(price)) => {
                let normalized = if price > dec!(1_000_000) {
                    price / dec!(1_000_000)
                } else {
                    price
                };
                Ok(normalized)
            }
            _ => anyhow::bail!("No price data available for market_index={}", market_index),
        }
    }

    /// Query open positions from the gateway
    pub async fn get_positions(&self) -> Result<Vec<GatewayPosition>> {
        let url = format!("{}/v2/positions", self.gateway_url);

        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("GET /v2/positions failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Get positions failed: {}", body);
        }

        let positions: Vec<GatewayPosition> = resp.json().await
            .context("Failed to parse positions response")?;

        Ok(positions)
    }

    /// Submit an order to the Drift Gateway with retry
    async fn submit_order(&self, order: &GatewayOrderRequest) -> Result<String> {
        let url = format!("{}/v2/orders", self.gateway_url);

        let mut last_err = None;

        for attempt in 1..=3 {
            let resp = match self.client
                .post(&url)
                .json(order)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Drift order attempt {}/3 failed: {}", attempt, e);
                    last_err = Some(e.into());
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
            };

            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            if status.is_success() {
                let parsed: GatewayOrderResponse = serde_json::from_str(&body)
                    .unwrap_or(GatewayOrderResponse {
                        tx: Some(body.clone()),
                        status: Some("ok".to_string()),
                    });

                let sig = parsed.tx
                    .unwrap_or_else(|| format!("order-{}", chrono::Utc::now().timestamp()));

                return Ok(sig);
            }

            warn!(
                "Drift order attempt {}/3 returned {}: {}",
                attempt, status, body
            );
            last_err = Some(anyhow::anyhow!("HTTP {}: {}", status, body));

            if status.as_u16() == 400 {
                // Client error — don't retry
                break;
            }

            tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Order submission failed")))
    }
}

fn dec_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}
