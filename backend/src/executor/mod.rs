pub mod drift_executor;
pub mod jupiter;

use anyhow::Result;
use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::types::*;
use crate::wallet::SolWallet;
use self::drift_executor::DriftExecutor;
use self::jupiter::JupiterClient;

pub struct TradeExecutor {
    drift: DriftExecutor,
    jupiter: JupiterClient,
    wallet: Arc<SolWallet>,
    dry_run: bool,
    take_profit_pct: Decimal,
    stop_loss_pct: Decimal,
}

impl TradeExecutor {
    pub fn new(wallet: Arc<SolWallet>, config: &AgentConfig) -> Self {
        let drift = DriftExecutor::new(
            Arc::clone(&wallet),
            &config.drift_api,
        );
        let jupiter = JupiterClient::new(
            Arc::clone(&wallet),
            &config.jupiter_api,
        );

        Self {
            drift,
            jupiter,
            wallet,
            dry_run: config.dry_run,
            take_profit_pct: config.take_profit_pct,
            stop_loss_pct: config.stop_loss_pct,
        }
    }

    /// Check if the Drift Gateway is reachable (for live mode)
    pub async fn check_gateway(&self) -> bool {
        if self.dry_run {
            return true;
        }
        match self.drift.health_check().await {
            Ok(healthy) => {
                if healthy {
                    info!("Drift Gateway: connected");
                } else {
                    warn!("Drift Gateway: returned unhealthy status");
                }
                healthy
            }
            Err(e) => {
                warn!("Drift Gateway: unreachable ({})", e);
                false
            }
        }
    }

    /// Execute a full arbitrage: Jupiter swap leg + Drift perp leg
    pub async fn execute_opportunity(
        &self,
        opp: &ArbOpportunity,
        size_usdc: Decimal,
    ) -> Result<Position> {
        let side = if opp.buy_poly_yes {
            PositionSide::Short
        } else {
            PositionSide::Long
        };

        let entry_price = opp.drift_signal.mark_price;
        let spread_in_price = entry_price * opp.net_spread;

        let (take_profit_price, stop_loss_price) = match side {
            PositionSide::Short => (
                entry_price - spread_in_price * self.take_profit_pct,
                entry_price + spread_in_price * self.stop_loss_pct,
            ),
            PositionSide::Long => (
                entry_price + spread_in_price * self.take_profit_pct,
                entry_price - spread_in_price * self.stop_loss_pct,
            ),
        };

        let position_id = Uuid::new_v4().to_string();

        info!(
            "Executing: {} {} {} ${:.0} @ {:.2} | TP={:.2} SL={:.2} | dry_run={}",
            opp.asset, side, opp.direction, size_usdc, entry_price,
            take_profit_price, stop_loss_price, self.dry_run,
        );

        let tx_sig = if self.dry_run {
            info!("[DRY RUN] Leg 1: Jupiter swap ${:.0} for {} exposure", size_usdc, opp.asset);
            info!("[DRY RUN] Leg 2: Drift {} {} on market {}", side, opp.asset, opp.drift_signal.market_index);
            Some(format!("dry-run-{}", &position_id[..8]))
        } else {
            // ── Leg 1: Jupiter swap for spot exposure ────────────────────
            // Swap USDC to the target asset for the Polymarket side
            if let Err(e) = self.execute_jupiter_leg(opp, size_usdc).await {
                warn!("Jupiter swap leg failed: {} — proceeding with Drift only", e);
                // Non-fatal: we can still open the Drift hedge
            }

            // ── Leg 2: Drift perpetual position ─────────────────────────
            match self.drift.open_perp_position(
                &opp.asset,
                &side,
                size_usdc,
                opp.drift_signal.market_index,
            ).await {
                Ok(sig) => {
                    info!("Drift order placed: {}", sig);
                    Some(sig)
                }
                Err(e) => {
                    warn!("Drift execution failed: {}", e);
                    return Err(e);
                }
            }
        };

        Ok(Position {
            id: position_id,
            opportunity_id: opp.id.clone(),
            asset: opp.asset.clone(),
            side,
            entry_price,
            size_usdc,
            drift_market_index: opp.drift_signal.market_index,
            take_profit_price,
            stop_loss_price,
            status: PositionStatus::Open,
            opened_at: Utc::now(),
            closed_at: None,
            pnl: None,
            tx_open: tx_sig,
            tx_close: None,
        })
    }

    /// Execute the Jupiter swap leg of the arbitrage.
    /// Swaps USDC to the target asset (or vice versa) depending on arb direction.
    async fn execute_jupiter_leg(
        &self,
        opp: &ArbOpportunity,
        size_usdc: Decimal,
    ) -> Result<jupiter::SwapResult> {
        let usdc_mint = self.wallet.usdc_mint_str();
        let target_mint = asset_to_mint(&opp.asset);

        // Convert USDC amount to smallest unit (6 decimals)
        let amount_raw = (size_usdc * dec!(1_000_000))
            .to_string()
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0);

        let slippage_bps = 50; // 0.5% slippage tolerance

        if opp.buy_poly_yes {
            // Buy asset on spot (Polymarket underpriced UP) + short on Drift
            info!(
                "Jupiter Leg: swapping ${:.0} USDC -> {} (spot buy)",
                size_usdc, opp.asset
            );
            let quote = self.jupiter.get_quote(
                usdc_mint,
                target_mint,
                amount_raw,
                slippage_bps,
            ).await?;
            self.jupiter.execute_swap(quote).await
        } else {
            // Sell asset on spot (Drift underpriced UP) + long on Drift
            // First check if we have the target asset to sell
            info!(
                "Jupiter Leg: swapping {} -> USDC (spot sell, ${:.0} equiv)",
                opp.asset, size_usdc
            );
            // Get quote for the reverse direction
            let quote = self.jupiter.get_quote(
                target_mint,
                usdc_mint,
                amount_raw, // approximate — Jupiter will quote the exact output
                slippage_bps,
            ).await?;
            self.jupiter.execute_swap(quote).await
        }
    }

    /// Close a position: reverse Drift perp + unwind Jupiter swap
    pub async fn close_position(&self, pos: &Position) -> Result<Decimal> {
        info!("Closing position: {}", pos);

        if self.dry_run {
            let simulated_pnl = pos.size_usdc * dec!(0.02);
            info!("[DRY RUN] Would close {} {} | Simulated PnL: ${:.2}", pos.side, pos.asset, simulated_pnl);
            return Ok(simulated_pnl);
        }

        // Close Drift perp position
        let sig = self.drift.close_perp_position(pos.drift_market_index).await?;
        info!("Drift close tx: {}", sig);

        // Unwind Jupiter swap leg (sell back the spot asset)
        if let Err(e) = self.unwind_jupiter_leg(pos).await {
            warn!("Jupiter unwind failed: {} — PnL may be approximate", e);
        }

        // Estimate PnL from mark price vs entry
        let current = self.drift.get_mark_price(pos.drift_market_index).await
            .unwrap_or(pos.entry_price);

        let pnl = match pos.side {
            PositionSide::Long => {
                (current - pos.entry_price) / pos.entry_price * pos.size_usdc
            }
            PositionSide::Short => {
                (pos.entry_price - current) / pos.entry_price * pos.size_usdc
            }
        };

        info!("Realized PnL: ${:.2}", pnl);
        Ok(pnl)
    }

    /// Reverse the Jupiter swap when closing a position
    async fn unwind_jupiter_leg(&self, pos: &Position) -> Result<jupiter::SwapResult> {
        let usdc_mint = self.wallet.usdc_mint_str();
        let target_mint = asset_to_mint(&pos.asset);
        let slippage_bps = 50;

        // Convert position size to raw amount
        let amount_raw = (pos.size_usdc * dec!(1_000_000))
            .to_string()
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0);

        match pos.side {
            PositionSide::Short => {
                // We bought spot + shorted perp → sell spot back to USDC
                info!("Jupiter unwind: selling {} -> USDC", pos.asset);
                let quote = self.jupiter.get_quote(
                    target_mint, usdc_mint, amount_raw, slippage_bps,
                ).await?;
                self.jupiter.execute_swap(quote).await
            }
            PositionSide::Long => {
                // We sold spot + longed perp → buy spot back
                info!("Jupiter unwind: buying {} with USDC", pos.asset);
                let quote = self.jupiter.get_quote(
                    usdc_mint, target_mint, amount_raw, slippage_bps,
                ).await?;
                self.jupiter.execute_swap(quote).await
            }
        }
    }

    pub fn check_exit_conditions(
        &self,
        pos: &Position,
        current_price: Decimal,
    ) -> Option<ExitReason> {
        match pos.side {
            PositionSide::Long => {
                if current_price >= pos.take_profit_price {
                    Some(ExitReason::TakeProfit)
                } else if current_price <= pos.stop_loss_price {
                    Some(ExitReason::StopLoss)
                } else {
                    None
                }
            }
            PositionSide::Short => {
                if current_price <= pos.take_profit_price {
                    Some(ExitReason::TakeProfit)
                } else if current_price >= pos.stop_loss_price {
                    Some(ExitReason::StopLoss)
                } else {
                    None
                }
            }
        }
    }
}

/// Map asset to its Solana SPL token mint address
fn asset_to_mint(asset: &Asset) -> &'static str {
    match asset {
        // Wrapped BTC on Solana (Portal)
        Asset::BTC => "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh",
        // Wrapped ETH on Solana (Portal)
        Asset::ETH => "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
        // Native SOL (wrapped)
        Asset::SOL => jupiter::SOL_MINT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pos(side: PositionSide, entry: Decimal, tp: Decimal, sl: Decimal) -> Position {
        Position {
            id: "test".to_string(),
            opportunity_id: "opp".to_string(),
            asset: Asset::BTC,
            side,
            entry_price: entry,
            size_usdc: dec!(500),
            drift_market_index: 1,
            take_profit_price: tp,
            stop_loss_price: sl,
            status: PositionStatus::Open,
            opened_at: Utc::now(),
            closed_at: None,
            pnl: None,
            tx_open: None,
            tx_close: None,
        }
    }

    #[test]
    fn test_long_take_profit() {
        let executor = TradeExecutor {
            drift: DriftExecutor::new_dry(),
            jupiter: JupiterClient::new_dry(),
            wallet: Arc::new(
                SolWallet::from_keypair(
                    solana_sdk::signature::Keypair::new(),
                    "https://api.devnet.solana.com",
                    SolanaNetwork::Devnet,
                ).unwrap()
            ),
            dry_run: true,
            take_profit_pct: dec!(0.50),
            stop_loss_pct: dec!(1.00),
        };

        let pos = make_pos(PositionSide::Long, dec!(65000), dec!(66000), dec!(64000));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(66500)), Some(ExitReason::TakeProfit));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(63500)), Some(ExitReason::StopLoss));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(65500)), None);
    }

    #[test]
    fn test_short_take_profit() {
        let executor = TradeExecutor {
            drift: DriftExecutor::new_dry(),
            jupiter: JupiterClient::new_dry(),
            wallet: Arc::new(
                SolWallet::from_keypair(
                    solana_sdk::signature::Keypair::new(),
                    "https://api.devnet.solana.com",
                    SolanaNetwork::Devnet,
                ).unwrap()
            ),
            dry_run: true,
            take_profit_pct: dec!(0.50),
            stop_loss_pct: dec!(1.00),
        };

        let pos = make_pos(PositionSide::Short, dec!(65000), dec!(64000), dec!(66000));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(63500)), Some(ExitReason::TakeProfit));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(66500)), Some(ExitReason::StopLoss));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(65500)), None);
    }

    #[test]
    fn test_asset_to_mint() {
        assert_eq!(asset_to_mint(&Asset::SOL), jupiter::SOL_MINT);
        assert!(!asset_to_mint(&Asset::BTC).is_empty());
        assert!(!asset_to_mint(&Asset::ETH).is_empty());
    }
}
