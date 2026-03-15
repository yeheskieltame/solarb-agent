use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::fmt;
use tracing::{info, warn};

use crate::types::{Asset, Position, PositionStatus, RiskLimits};

// ── Risk denial reasons ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskDenial {
    MaxPositionExceeded { requested: Decimal, limit: Decimal },
    MaxExposureExceeded { current: Decimal, requested: Decimal, limit: Decimal },
    MaxOpenPositionsExceeded { current: usize, limit: usize },
    DailyLossStopTriggered { daily_pnl: Decimal, limit: Decimal },
    DuplicateAsset { asset: Asset },
    InsufficientBalance { available: Decimal, required: Decimal },
}

impl fmt::Display for RiskDenial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxPositionExceeded { requested, limit } =>
                write!(f, "Position ${} exceeds max ${}", requested, limit),
            Self::MaxExposureExceeded { current, requested, limit } =>
                write!(f, "Exposure ${} + ${} exceeds max ${}", current, requested, limit),
            Self::MaxOpenPositionsExceeded { current, limit } =>
                write!(f, "Open positions {} >= max {}", current, limit),
            Self::DailyLossStopTriggered { daily_pnl, limit } =>
                write!(f, "Daily PnL ${} hit loss stop -${}", daily_pnl, limit),
            Self::DuplicateAsset { asset } =>
                write!(f, "Already have open position on {}", asset),
            Self::InsufficientBalance { available, required } =>
                write!(f, "Balance ${} < required ${}", available, required),
        }
    }
}

// ── Risk manager ─────────────────────────────────────────────────────────────

pub struct RiskManager {
    pub limits: RiskLimits,
    positions: Vec<Position>,
    daily_pnl: Decimal,
    day_marker: NaiveDate,
}

impl RiskManager {
    pub fn new(limits: RiskLimits) -> Self {
        Self {
            limits,
            positions: Vec::new(),
            daily_pnl: dec!(0),
            day_marker: Utc::now().date_naive(),
        }
    }

    fn maybe_reset_daily(&mut self) {
        let today = Utc::now().date_naive();
        if today > self.day_marker {
            info!("New day — resetting daily PnL (was ${:.2})", self.daily_pnl);
            self.daily_pnl = dec!(0);
            self.day_marker = today;
        }
    }

    pub fn can_open(&mut self, size_usdc: Decimal, asset: &Asset) -> Result<(), RiskDenial> {
        self.maybe_reset_daily();

        if size_usdc > self.limits.max_position_usdc {
            return Err(RiskDenial::MaxPositionExceeded {
                requested: size_usdc,
                limit: self.limits.max_position_usdc,
            });
        }

        let exposure = self.total_open_exposure();
        if exposure + size_usdc > self.limits.max_total_exposure_usdc {
            return Err(RiskDenial::MaxExposureExceeded {
                current: exposure,
                requested: size_usdc,
                limit: self.limits.max_total_exposure_usdc,
            });
        }

        let open_count = self.open_positions().len();
        if open_count >= self.limits.max_open_positions {
            return Err(RiskDenial::MaxOpenPositionsExceeded {
                current: open_count,
                limit: self.limits.max_open_positions,
            });
        }

        if self.daily_pnl < -self.limits.daily_loss_stop_usdc {
            return Err(RiskDenial::DailyLossStopTriggered {
                daily_pnl: self.daily_pnl,
                limit: self.limits.daily_loss_stop_usdc,
            });
        }

        if self.find_position_for_asset(asset).is_some() {
            return Err(RiskDenial::DuplicateAsset { asset: asset.clone() });
        }

        Ok(())
    }

    pub fn size_for_opportunity(
        &self,
        max_position: Decimal,
        liquidity: Decimal,
        available_balance: Decimal,
    ) -> Decimal {
        let headroom = self.limits.max_total_exposure_usdc - self.total_open_exposure();
        max_position
            .min(liquidity)
            .min(available_balance)
            .min(headroom)
            .max(dec!(0))
    }

    pub fn open_position(&mut self, pos: Position) {
        info!("Position opened: {}", pos);
        self.positions.push(pos);
    }

    pub fn close_position(&mut self, position_id: &str, pnl: Decimal) {
        self.maybe_reset_daily();
        self.daily_pnl += pnl;

        if let Some(pos) = self.positions.iter_mut().find(|p| p.id == position_id) {
            pos.status = PositionStatus::Closed;
            pos.closed_at = Some(Utc::now());
            pos.pnl = Some(pnl);
            info!("Position closed: {} | PnL: ${:.2} | Daily: ${:.2}", position_id, pnl, self.daily_pnl);
        } else {
            warn!("Tried to close unknown position: {}", position_id);
        }
    }

    pub fn open_positions(&self) -> Vec<&Position> {
        self.positions.iter()
            .filter(|p| p.status == PositionStatus::Open || p.status == PositionStatus::Opening)
            .collect()
    }

    pub fn total_open_exposure(&self) -> Decimal {
        self.open_positions().iter().map(|p| p.size_usdc).sum()
    }

    pub fn find_position_for_asset(&self, asset: &Asset) -> Option<&Position> {
        self.open_positions().into_iter().find(|p| p.asset == *asset)
    }

    pub fn daily_pnl(&self) -> Decimal {
        self.daily_pnl
    }

    pub fn all_positions(&self) -> &[Position] {
        &self.positions
    }

    pub fn log_summary(&self) {
        let open = self.open_positions();
        info!(
            "Risk | open={} exposure=${:.0} daily_pnl=${:.2} total_trades={}",
            open.len(),
            self.total_open_exposure(),
            self.daily_pnl,
            self.positions.len(),
        );
        for pos in &open {
            info!("  {}", pos);
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;

    fn make_position(asset: Asset, size: Decimal) -> Position {
        Position {
            id: format!("test-{}", asset),
            opportunity_id: "opp-1".to_string(),
            asset,
            side: PositionSide::Long,
            entry_price: dec!(65000),
            size_usdc: size,
            drift_market_index: 1,
            take_profit_price: dec!(66000),
            stop_loss_price: dec!(64000),
            status: PositionStatus::Open,
            opened_at: Utc::now(),
            closed_at: None,
            pnl: None,
            tx_open: Some("sig123".to_string()),
            tx_close: None,
        }
    }

    #[test]
    fn test_can_open_basic() {
        let mut rm = RiskManager::new(RiskLimits::default());
        assert!(rm.can_open(dec!(200), &Asset::BTC).is_ok());
    }

    #[test]
    fn test_max_position_exceeded() {
        let mut rm = RiskManager::new(RiskLimits {
            max_position_usdc: dec!(100),
            ..Default::default()
        });
        let result = rm.can_open(dec!(200), &Asset::BTC);
        assert!(matches!(result, Err(RiskDenial::MaxPositionExceeded { .. })));
    }

    #[test]
    fn test_max_exposure_exceeded() {
        let mut rm = RiskManager::new(RiskLimits {
            max_total_exposure_usdc: dec!(500),
            ..Default::default()
        });
        rm.open_position(make_position(Asset::BTC, dec!(400)));
        let result = rm.can_open(dec!(200), &Asset::ETH);
        assert!(matches!(result, Err(RiskDenial::MaxExposureExceeded { .. })));
    }

    #[test]
    fn test_duplicate_asset() {
        let mut rm = RiskManager::new(RiskLimits::default());
        rm.open_position(make_position(Asset::BTC, dec!(100)));
        let result = rm.can_open(dec!(100), &Asset::BTC);
        assert!(matches!(result, Err(RiskDenial::DuplicateAsset { .. })));
    }

    #[test]
    fn test_daily_loss_stop() {
        let mut rm = RiskManager::new(RiskLimits {
            daily_loss_stop_usdc: dec!(100),
            ..Default::default()
        });
        rm.open_position(make_position(Asset::BTC, dec!(200)));
        rm.close_position("test-BTC", dec!(-150));
        let result = rm.can_open(dec!(100), &Asset::ETH);
        assert!(matches!(result, Err(RiskDenial::DailyLossStopTriggered { .. })));
    }

    #[test]
    fn test_size_for_opportunity() {
        let rm = RiskManager::new(RiskLimits {
            max_total_exposure_usdc: dec!(1000),
            ..Default::default()
        });
        let size = rm.size_for_opportunity(dec!(500), dec!(300), dec!(800));
        assert_eq!(size, dec!(300)); // limited by liquidity
    }

    #[test]
    fn test_close_updates_pnl() {
        let mut rm = RiskManager::new(RiskLimits::default());
        rm.open_position(make_position(Asset::BTC, dec!(100)));
        rm.close_position("test-BTC", dec!(25));
        assert_eq!(rm.daily_pnl(), dec!(25));
        assert_eq!(rm.open_positions().len(), 0);
    }
}
