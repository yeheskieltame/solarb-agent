use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::types::{Asset, PositionSide};
use crate::wallet::SolWallet;

// ── Drift order types ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaceOrderRequest {
    market_index: u16,
    market_type: String,
    side: String,
    amount: String,
    order_type: String,
    reduce_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderResponse {
    tx_sig: Option<String>,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerpPositionResponse {
    base_asset_amount: Option<String>,
    quote_entry_amount: Option<String>,
    unrealized_pnl: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkPriceResponse {
    data: Option<MarkPriceData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkPriceData {
    mark_price: Option<String>,
}

// ── Drift executor ───────────────────────────────────────────────────────────

pub struct DriftExecutor {
    client: Client,
    base_url: String,
    wallet: Option<Arc<SolWallet>>,
}

impl DriftExecutor {
    pub fn new(wallet: Arc<SolWallet>, base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("SolArb-Agent/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.to_string(),
            wallet: Some(wallet),
        }
    }

    pub fn new_dry() -> Self {
        Self {
            client: Client::new(),
            base_url: String::new(),
            wallet: None,
        }
    }

    pub async fn open_perp_position(
        &self,
        asset: &Asset,
        side: &PositionSide,
        size_usdc: Decimal,
        market_index: u16,
    ) -> Result<String> {
        let side_str = match side {
            PositionSide::Long => "long",
            PositionSide::Short => "short",
        };

        info!(
            "Drift: opening {} {} perp, size=${}, market_index={}",
            side_str, asset, size_usdc, market_index
        );

        let order = PlaceOrderRequest {
            market_index,
            market_type: "perp".to_string(),
            side: side_str.to_string(),
            amount: size_usdc.to_string(),
            order_type: "market".to_string(),
            reduce_only: false,
        };

        let url = format!("{}/v2/orders", self.base_url);

        let resp = self.client
            .post(&url)
            .json(&order)
            .send()
            .await
            .context("POST /v2/orders failed")?;

        let status = resp.status();
        let body = resp.text().await.context("Failed to read response")?;

        if !status.is_success() {
            anyhow::bail!("Drift order failed ({}): {}", status, body);
        }

        let parsed: OrderResponse = serde_json::from_str(&body)
            .context("Failed to parse order response")?;

        let sig = parsed.tx_sig.unwrap_or_else(|| format!("pending-{}", market_index));
        info!("Drift order submitted: {} (status: {})", sig, parsed.status);

        Ok(sig)
    }

    pub async fn close_perp_position(&self, market_index: u16) -> Result<String> {
        info!("Drift: closing perp position on market_index={}", market_index);

        let order = PlaceOrderRequest {
            market_index,
            market_type: "perp".to_string(),
            side: "long".to_string(), // direction doesn't matter for reduce_only
            amount: "0".to_string(),  // close full position
            order_type: "market".to_string(),
            reduce_only: true,
        };

        let url = format!("{}/v2/orders", self.base_url);

        let resp = self.client
            .post(&url)
            .json(&order)
            .send()
            .await
            .context("POST /v2/orders (close) failed")?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            anyhow::bail!("Drift close failed ({}): {}", status, body);
        }

        let parsed: OrderResponse = serde_json::from_str(&body)?;
        let sig = parsed.tx_sig.unwrap_or_else(|| "close-pending".to_string());
        info!("Drift close submitted: {}", sig);

        Ok(sig)
    }

    pub async fn get_mark_price(&self, market_index: u16) -> Result<Decimal> {
        let url = format!("{}/v2/perpMarkets/{}", self.base_url, market_index);

        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("GET mark price failed")?;

        let body: MarkPriceResponse = resp.json().await
            .context("Failed to parse mark price")?;

        let price_str = body.data
            .and_then(|d| d.mark_price)
            .ok_or_else(|| anyhow::anyhow!("No mark price in response"))?;

        // Drift prices are scaled by 1e6
        let raw: Decimal = price_str.parse().context("bad mark price")?;
        Ok(raw / dec!(1_000_000))
    }
}
