use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::wallet::SolWallet;

// ── Jupiter V6 API types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterQuote {
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub swap_mode: String,
    pub slippage_bps: u16,
    pub price_impact_pct: String,
    pub route_plan: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SwapRequest {
    quote_response: JupiterQuote,
    user_public_key: String,
    wrap_and_unwrap_sol: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwapResponse {
    swap_transaction: String,
}

// ── Jupiter client ───────────────────────────────────────────────────────────

pub struct JupiterClient {
    client: Client,
    api_url: String,
    wallet: Option<Arc<SolWallet>>,
}

impl JupiterClient {
    pub fn new(wallet: Arc<SolWallet>, api_url: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("SolArb-Agent/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            api_url: api_url.to_string(),
            wallet: Some(wallet),
        }
    }

    pub fn new_dry() -> Self {
        Self {
            client: Client::new(),
            api_url: String::new(),
            wallet: None,
        }
    }

    pub async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<JupiterQuote> {
        let url = format!(
            "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            self.api_url, input_mint, output_mint, amount, slippage_bps
        );

        debug!("Jupiter quote: {}", url);

        let resp = self.client
            .get(&url)
            .send()
            .await
            .context("Jupiter GET /quote failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Jupiter quote failed ({}): {}", status, body);
        }

        let quote: JupiterQuote = resp.json().await
            .context("Failed to parse Jupiter quote")?;

        info!(
            "Jupiter quote: {} {} -> {} {} (impact: {}%)",
            quote.in_amount, input_mint,
            quote.out_amount, output_mint,
            quote.price_impact_pct,
        );

        Ok(quote)
    }

    pub async fn execute_swap(&self, quote: JupiterQuote) -> Result<String> {
        let wallet = self.wallet.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No wallet configured for swap"))?;

        let swap_req = SwapRequest {
            quote_response: quote,
            user_public_key: wallet.pubkey().to_string(),
            wrap_and_unwrap_sol: true,
        };

        let url = format!("{}/swap", self.api_url);

        let resp = self.client
            .post(&url)
            .json(&swap_req)
            .send()
            .await
            .context("Jupiter POST /swap failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Jupiter swap failed ({}): {}", status, body);
        }

        let swap_resp: SwapResponse = resp.json().await
            .context("Failed to parse swap response")?;

        // Deserialize, sign, and send the transaction
        let tx_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &swap_resp.swap_transaction,
        ).context("Failed to decode swap transaction")?;

        let mut tx: solana_sdk::transaction::VersionedTransaction =
            bincode::deserialize(&tx_bytes)
                .context("Failed to deserialize swap transaction")?;

        info!("Jupiter swap tx ready, signing and sending...");

        // For versioned transactions, we need to sign differently
        let recent_blockhash = wallet.rpc
            .get_latest_blockhash()
            .await
            .context("Failed to get blockhash")?;

        tx.message.set_recent_blockhash(recent_blockhash);

        let sig = wallet.rpc
            .send_and_confirm_transaction(&tx.into_legacy_transaction().context("Not a legacy tx")?)
            .await
            .context("Jupiter swap tx failed")?;

        info!("Jupiter swap confirmed: {}", sig);
        Ok(sig.to_string())
    }
}
