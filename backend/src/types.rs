use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Market identifiers ────────────────────────────────────────────────────────

/// Which underlying asset this signal is about
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Asset {
    BTC,
    ETH,
    SOL,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Asset::BTC => write!(f, "BTC"),
            Asset::ETH => write!(f, "ETH"),
            Asset::SOL => write!(f, "SOL"),
        }
    }
}

/// Direction of the prediction / trade
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,   // price will be higher at resolution
    Down, // price will be lower at resolution
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::Up => write!(f, "UP"),
            Direction::Down => write!(f, "DOWN"),
        }
    }
}

// ── Raw price signals from each venue ────────────────────────────────────────

/// A single Polymarket market snapshot
/// YES token price ≈ market's implied probability that event resolves YES
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketSignal {
    pub asset: Asset,
    pub direction: Direction,
    /// Market resolution time (UTC)
    pub resolves_at: DateTime<Utc>,
    /// Best bid for YES token (0.00–1.00)
    pub yes_bid: Decimal,
    /// Best ask for YES token (0.00–1.00)
    pub yes_ask: Decimal,
    /// Mid-price — what we use as "Polymarket's implied probability"
    pub yes_mid: Decimal,
    /// Available liquidity in USDC on the YES side
    pub yes_liquidity: Decimal,
    /// Polymarket condition_id for this market
    pub condition_id: String,
    /// Token id for the YES outcome token
    pub yes_token_id: String,
    pub captured_at: DateTime<Utc>,
}

impl PolymarketSignal {
    /// Returns the mid-price as a probability (0.0–1.0)
    pub fn implied_probability(&self) -> Decimal {
        self.yes_mid
    }
}

/// Drift Protocol perpetual funding rate snapshot
/// Positive funding rate = longs pay shorts (market skewed long / bullish sentiment)
/// Negative funding rate = shorts pay longs (market skewed short / bearish sentiment)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftSignal {
    pub asset: Asset,
    /// Current 1-hour funding rate (e.g., 0.0001 = 0.01%/hr)
    pub funding_rate_1h: Decimal,
    /// Mark price from Drift
    pub mark_price: Decimal,
    /// Oracle (spot reference) price
    pub oracle_price: Decimal,
    /// Mark premium = (mark - oracle) / oracle
    /// Positive = futures trading at premium → market expects price rise
    pub mark_premium: Decimal,
    /// Drift market index
    pub market_index: u16,
    pub captured_at: DateTime<Utc>,
}

impl DriftSignal {
    /// Convert Drift's funding rate + mark premium into a directional
    /// implied probability over the next N hours.
    ///
    /// This is a heuristic model:
    ///   - mark_premium > 0 → market leans UP (probability > 0.5)
    ///   - funding_rate > 0 → longs are paying → also bullish signal
    ///   - We blend both with a sigmoid-style normalisation
    pub fn implied_up_probability(&self, horizon_hours: u32) -> Decimal {
        // Scale the mark premium: 1% premium → roughly +5% probability shift
        let premium_contribution = self.mark_premium * Decimal::new(5, 0);

        // Funding rate accumulated over horizon: positive funding → bullish signal
        // Scale: 0.01%/hr * 24hr = 0.24% → roughly +2% probability shift
        let funding_contribution =
            self.funding_rate_1h * Decimal::new(horizon_hours as i64, 0) * Decimal::new(20, 0);

        let raw_shift = premium_contribution + funding_contribution;

        // Clamp shift to ±25% — extreme values are unreliable
        let shift = raw_shift
            .max(Decimal::new(-25, 2))
            .min(Decimal::new(25, 2));

        // Base probability of 0.5 + shift
        (Decimal::new(5, 1) + shift)
            .max(Decimal::new(1, 2))  // floor at 1%
            .min(Decimal::new(99, 2)) // cap at 99%
    }
}

// ── Arbitrage opportunity ─────────────────────────────────────────────────────

/// Confidence level of an arbitrage opportunity
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    Low,    // 2.5–3.5% net spread — marginal
    Medium, // 3.5–6% net spread — actionable
    High,   // >6% net spread — strong signal
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Confidence::Low => write!(f, "LOW"),
            Confidence::Medium => write!(f, "MEDIUM"),
            Confidence::High => write!(f, "HIGH"),
        }
    }
}

/// A detected arbitrage opportunity — the output of our scanner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbOpportunity {
    pub id: String,               // unique: "{asset}-{direction}-{timestamp}"
    pub asset: Asset,
    pub direction: Direction,

    // Polymarket side
    pub poly_signal: PolymarketSignal,
    /// Implied probability from Polymarket (0.0–1.0)
    pub poly_prob: Decimal,

    // Drift side
    pub drift_signal: DriftSignal,
    /// Implied probability from Drift (0.0–1.0)
    pub drift_prob: Decimal,

    // Spread analysis
    /// Raw probability spread: |poly_prob - drift_prob|
    pub gross_spread: Decimal,
    /// Polymarket taker fee (dynamic, ~2–3% on 50-cent contracts)
    pub poly_fee: Decimal,
    /// Drift trading fee estimate
    pub drift_fee: Decimal,
    /// Net spread after all fees — must be > 0 to be profitable
    pub net_spread: Decimal,

    // Trade direction
    /// true = buy YES on Polymarket + short on Drift (Poly underpriced)
    /// false = buy NO on Polymarket + long on Drift (Drift underpriced)
    pub buy_poly_yes: bool,

    // Risk
    pub confidence: Confidence,
    /// Available liquidity on the thinner side (limiting factor)
    pub liquidity_usdc: Decimal,
    /// Time until Polymarket market resolves
    pub time_to_resolution_mins: i64,

    pub detected_at: DateTime<Utc>,
}

impl ArbOpportunity {
    /// Estimated profit in USDC for a given position size
    pub fn estimated_profit(&self, position_usdc: Decimal) -> Decimal {
        self.net_spread * position_usdc
    }

    /// Whether this opportunity is worth acting on
    pub fn is_actionable(&self) -> bool {
        self.net_spread > Decimal::new(25, 3) // net spread > 2.5%
            && self.liquidity_usdc >= Decimal::new(100, 0) // at least $100 liquidity
            && self.time_to_resolution_mins > 2 // at least 2 minutes to act
    }
}

impl fmt::Display for ArbOpportunity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{conf}] {asset} {dir} | Poly: {poly:.1}% vs Drift: {drift:.1}% | Net spread: {net:.2}% | Liquidity: ${liq:.0} | T-{mins}min",
            conf = self.confidence,
            asset = self.asset,
            dir = self.direction,
            poly = self.poly_prob * Decimal::new(100, 0),
            drift = self.drift_prob * Decimal::new(100, 0),
            net = self.net_spread * Decimal::new(100, 0),
            liq = self.liquidity_usdc,
            mins = self.time_to_resolution_mins,
        )
    }
}

// ── Position tracking (Sprint 2) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionSide {
    Long,
    Short,
}

impl fmt::Display for PositionSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PositionSide::Long => write!(f, "LONG"),
            PositionSide::Short => write!(f, "SHORT"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    Opening,
    Open,
    Closing,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub opportunity_id: String,
    pub asset: Asset,
    pub side: PositionSide,
    pub entry_price: Decimal,
    pub size_usdc: Decimal,
    pub drift_market_index: u16,
    pub take_profit_price: Decimal,
    pub stop_loss_price: Decimal,
    pub status: PositionStatus,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub pnl: Option<Decimal>,
    pub tx_open: Option<String>,
    pub tx_close: Option<String>,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} ${:.0} @ {:.2} | TP={:.2} SL={:.2} | {:?}",
            self.asset, self.side, self.size_usdc, self.entry_price,
            self.take_profit_price, self.stop_loss_price, self.status,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    TakeProfit,
    StopLoss,
    Expired,
}

impl fmt::Display for ExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExitReason::TakeProfit => write!(f, "TAKE_PROFIT"),
            ExitReason::StopLoss => write!(f, "STOP_LOSS"),
            ExitReason::Expired => write!(f, "EXPIRED"),
        }
    }
}

// ── Risk ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RiskLimits {
    pub max_position_usdc: Decimal,
    pub max_total_exposure_usdc: Decimal,
    pub daily_loss_stop_usdc: Decimal,
    pub max_open_positions: usize,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_position_usdc: Decimal::new(500, 0),
            max_total_exposure_usdc: Decimal::new(2000, 0),
            daily_loss_stop_usdc: Decimal::new(200, 0),
            max_open_positions: 5,
        }
    }
}

// ── Network ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolanaNetwork {
    Devnet,
    Mainnet,
}

impl SolanaNetwork {
    pub fn usdc_mint(&self) -> &str {
        match self {
            SolanaNetwork::Devnet => "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
            SolanaNetwork::Mainnet => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        }
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub min_net_spread: Decimal,
    pub max_position_usdc: Decimal,
    pub max_total_exposure_usdc: Decimal,
    pub scan_interval_secs: u64,
    pub polymarket_api: String,
    pub drift_api: String,
    pub solana_rpc: String,
    pub network: SolanaNetwork,
    pub keypair_path: Option<String>,
    pub jupiter_api: String,
    pub dry_run: bool,
    pub daily_loss_stop_usdc: Decimal,
    pub take_profit_pct: Decimal,
    pub stop_loss_pct: Decimal,
    pub max_open_positions: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            min_net_spread: Decimal::new(25, 3),
            max_position_usdc: Decimal::new(500, 0),
            max_total_exposure_usdc: Decimal::new(2000, 0),
            scan_interval_secs: 3,
            polymarket_api: "https://clob.polymarket.com".to_string(),
            drift_api: "https://mainnet-beta.api.drift.trade".to_string(),
            solana_rpc: "https://api.mainnet-beta.solana.com".to_string(),
            network: SolanaNetwork::Devnet,
            keypair_path: None,
            jupiter_api: "https://quote-api.jup.ag/v6".to_string(),
            dry_run: true,
            daily_loss_stop_usdc: Decimal::new(200, 0),
            take_profit_pct: Decimal::new(50, 2),
            stop_loss_pct: Decimal::new(100, 2),
            max_open_positions: 5,
        }
    }
}
