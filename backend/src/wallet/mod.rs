use anyhow::{Context, Result};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::types::SolanaNetwork;

pub struct SolWallet {
    keypair: Keypair,
    pub rpc: RpcClient,
    usdc_mint: Pubkey,
    pub network: SolanaNetwork,
}

impl SolWallet {
    pub fn from_file(path: &str, rpc_url: &str, network: SolanaNetwork) -> Result<Self> {
        let key_bytes = std::fs::read_to_string(path)
            .context("Failed to read keypair file")?;
        let bytes: Vec<u8> = serde_json::from_str(&key_bytes)
            .context("Failed to parse keypair JSON (expected [u8; 64] array)")?;
        let keypair = Keypair::from_bytes(&bytes)
            .context("Invalid keypair bytes")?;

        let usdc_mint = Pubkey::from_str(network.usdc_mint())
            .context("Invalid USDC mint address")?;

        let rpc = RpcClient::new_with_commitment(
            rpc_url.to_string(),
            CommitmentConfig::confirmed(),
        );

        info!("Wallet loaded: {}", keypair.pubkey());

        Ok(Self { keypair, rpc, usdc_mint, network })
    }

    pub fn from_keypair(keypair: Keypair, rpc_url: &str, network: SolanaNetwork) -> Result<Self> {
        let usdc_mint = Pubkey::from_str(network.usdc_mint())
            .context("Invalid USDC mint address")?;

        let rpc = RpcClient::new_with_commitment(
            rpc_url.to_string(),
            CommitmentConfig::confirmed(),
        );

        Ok(Self { keypair, rpc, usdc_mint, network })
    }

    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub async fn sol_balance(&self) -> Result<Decimal> {
        let lamports = self.rpc
            .get_balance(&self.keypair.pubkey())
            .await
            .context("Failed to fetch SOL balance")?;
        Ok(Decimal::new(lamports as i64, 9))
    }

    pub async fn usdc_balance(&self) -> Result<Decimal> {
        let ata = spl_associated_token_account(&self.keypair.pubkey(), &self.usdc_mint);

        match self.rpc.get_token_account_balance(&ata).await {
            Ok(balance) => {
                let amount = balance.ui_amount.unwrap_or(0.0);
                Ok(Decimal::try_from(amount).unwrap_or(dec!(0)))
            }
            Err(_) => {
                debug!("USDC token account not found — balance = 0");
                Ok(dec!(0))
            }
        }
    }

    pub async fn send_and_confirm(&self, tx: Transaction) -> Result<Signature> {
        let sig = self.rpc
            .send_and_confirm_transaction(&tx)
            .await
            .context("Transaction failed")?;
        info!("Transaction confirmed: {}", sig);
        Ok(sig)
    }

    pub async fn log_balances(&self) {
        match self.sol_balance().await {
            Ok(sol) => info!("  SOL balance : {:.4} SOL", sol),
            Err(e) => warn!("  SOL balance : error ({})", e),
        }
        match self.usdc_balance().await {
            Ok(usdc) => info!("  USDC balance: ${:.2}", usdc),
            Err(e) => warn!("  USDC balance: error ({})", e),
        }
    }
}

/// Derive the associated token account address (matching spl-associated-token-account logic)
fn spl_associated_token_account(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let spl_token = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
    let ata_program = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();

    let seeds = &[
        wallet.as_ref(),
        spl_token.as_ref(),
        mint.as_ref(),
    ];

    Pubkey::find_program_address(seeds, &ata_program).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ata_derivation() {
        // Known wallet + USDC mint should produce a deterministic ATA
        let wallet = Pubkey::from_str("11111111111111111111111111111112").unwrap();
        let mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ata = spl_associated_token_account(&wallet, &mint);
        // Just verify it doesn't panic and produces a valid pubkey
        assert_ne!(ata, Pubkey::default());
    }
}
