use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use solana_sdk::signature::Signer;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::wallet::SolWallet;

// ── Well-known Solana token mints ───────────────────────────────────────────

pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDC_MINT_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

// ── Jupiter V6 API types ────────────────────────────────────────────────────

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

impl JupiterQuote {
    /// Parse output amount as Decimal
    pub fn out_amount_dec(&self) -> Decimal {
        self.out_amount.parse().unwrap_or(dec!(0))
    }

    /// Price impact as f64 percentage
    pub fn impact_pct(&self) -> f64 {
        self.price_impact_pct.parse().unwrap_or(0.0)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SwapRequest {
    quote_response: JupiterQuote,
    user_public_key: String,
    wrap_and_unwrap_sol: bool,
    /// Use versioned transactions for better compute budget
    #[serde(skip_serializing_if = "Option::is_none")]
    as_legacy_transaction: Option<bool>,
    /// Priority fee in micro-lamports
    #[serde(skip_serializing_if = "Option::is_none")]
    compute_unit_price_micro_lamports: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwapResponse {
    swap_transaction: String,
}

/// Result of a completed swap
#[derive(Debug)]
pub struct SwapResult {
    pub tx_signature: String,
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: String,
    pub out_amount: String,
}

// ── Jupiter client ───────────────────────────────────────────────────────────

pub struct JupiterClient {
    client: Client,
    api_url: String,
    wallet: Option<Arc<SolWallet>>,
    /// Max acceptable price impact in percent (e.g. 1.0 = 1%)
    max_price_impact_pct: f64,
}

impl JupiterClient {
    pub fn new(wallet: Arc<SolWallet>, api_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("SolArb-Agent/0.3")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            api_url: api_url.to_string(),
            wallet: Some(wallet),
            max_price_impact_pct: 1.0,
        }
    }

    pub fn new_dry() -> Self {
        Self {
            client: Client::new(),
            api_url: String::new(),
            wallet: None,
            max_price_impact_pct: 1.0,
        }
    }

    /// Get a swap quote from Jupiter V6
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
            "Jupiter quote: {} {} -> {} {} (impact: {}%, slippage: {}bps)",
            quote.in_amount, input_mint,
            quote.out_amount, output_mint,
            quote.price_impact_pct, quote.slippage_bps,
        );

        // Guard against excessive price impact
        if quote.impact_pct() > self.max_price_impact_pct {
            anyhow::bail!(
                "Jupiter price impact too high: {}% > {}% max",
                quote.price_impact_pct, self.max_price_impact_pct
            );
        }

        Ok(quote)
    }

    /// Execute a swap using a previously obtained quote.
    /// Handles versioned transactions with proper signing.
    pub async fn execute_swap(&self, quote: JupiterQuote) -> Result<SwapResult> {
        let wallet = self.wallet.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No wallet configured for Jupiter swap"))?;

        let in_amount = quote.in_amount.clone();
        let out_amount = quote.out_amount.clone();
        let input_mint = quote.input_mint.clone();
        let output_mint = quote.output_mint.clone();

        let swap_req = SwapRequest {
            quote_response: quote,
            user_public_key: wallet.pubkey().to_string(),
            wrap_and_unwrap_sol: true,
            as_legacy_transaction: Some(true), // legacy for reliable signing
            compute_unit_price_micro_lamports: Some(50_000), // priority fee
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

        // Decode the base64 transaction
        use base64::Engine;
        let tx_bytes = base64::engine::general_purpose::STANDARD
            .decode(&swap_resp.swap_transaction)
            .context("Failed to decode swap transaction base64")?;

        // Try versioned transaction first, fallback to legacy
        let sig = self.sign_and_send(wallet, &tx_bytes).await?;

        info!("Jupiter swap confirmed: {}", sig);

        Ok(SwapResult {
            tx_signature: sig,
            input_mint,
            output_mint,
            in_amount,
            out_amount,
        })
    }

    /// Sign and send a serialized transaction with retry logic
    async fn sign_and_send(&self, wallet: &SolWallet, tx_bytes: &[u8]) -> Result<String> {
        let recent_blockhash = wallet.rpc
            .get_latest_blockhash()
            .await
            .context("Failed to get recent blockhash")?;

        // Deserialize as legacy transaction (we request legacy from Jupiter)
        let mut tx: solana_sdk::transaction::Transaction =
            bincode::deserialize(tx_bytes)
                .context("Failed to deserialize swap transaction")?;

        tx.try_sign(&[wallet.keypair()], recent_blockhash)?;

        // Send with retry
        for attempt in 1..=3u64 {
            match wallet.rpc.send_and_confirm_transaction(&tx).await {
                Ok(sig) => {
                    info!("Jupiter swap confirmed (attempt {}): {}", attempt, sig);
                    return Ok(sig.to_string());
                }
                Err(e) => {
                    warn!("Jupiter send attempt {}/3 failed: {}", attempt, e);
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(500 * attempt)).await;

                        // Refresh blockhash for retry
                        if let Ok(bh) = wallet.rpc.get_latest_blockhash().await {
                            tx.try_sign(&[wallet.keypair()], bh)?;
                        }
                    }
                }
            }
        }

        anyhow::bail!("Jupiter swap failed after 3 attempts")
    }

    /// Convenience: swap USDC to SOL for gas fees
    pub async fn swap_usdc_to_sol(
        &self,
        usdc_amount: u64, // in USDC smallest units (1 USDC = 1_000_000)
        usdc_mint: &str,
        slippage_bps: u16,
    ) -> Result<SwapResult> {
        let quote = self.get_quote(usdc_mint, SOL_MINT, usdc_amount, slippage_bps).await?;
        self.execute_swap(quote).await
    }

    /// Convenience: swap SOL to USDC
    pub async fn swap_sol_to_usdc(
        &self,
        sol_lamports: u64,
        usdc_mint: &str,
        slippage_bps: u16,
    ) -> Result<SwapResult> {
        let quote = self.get_quote(SOL_MINT, usdc_mint, sol_lamports, slippage_bps).await?;
        self.execute_swap(quote).await
    }
}
