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
            dry_run: config.dry_run,
            take_profit_pct: config.take_profit_pct,
            stop_loss_pct: config.stop_loss_pct,
        }
    }

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
            info!("[DRY RUN] Would open {} {} on Drift market {}", side, opp.asset, opp.drift_signal.market_index);
            Some(format!("dry-run-{}", &position_id[..8]))
        } else {
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

    pub async fn close_position(&self, pos: &Position) -> Result<Decimal> {
        info!("Closing position: {}", pos);

        if self.dry_run {
            let simulated_pnl = pos.size_usdc * dec!(0.02);
            info!("[DRY RUN] Would close {} {} | Simulated PnL: ${:.2}", pos.side, pos.asset, simulated_pnl);
            return Ok(simulated_pnl);
        }

        let sig = self.drift.close_perp_position(pos.drift_market_index).await?;
        info!("Close tx: {}", sig);

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

    pub fn jupiter(&self) -> &JupiterClient {
        &self.jupiter
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
            dry_run: true,
            take_profit_pct: dec!(0.50),
            stop_loss_pct: dec!(1.00),
        };

        let pos = make_pos(PositionSide::Short, dec!(65000), dec!(64000), dec!(66000));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(63500)), Some(ExitReason::TakeProfit));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(66500)), Some(ExitReason::StopLoss));
        assert_eq!(executor.check_exit_conditions(&pos, dec!(65500)), None);
    }
}
