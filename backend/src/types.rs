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

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Minimum net spread required to flag as opportunity
    pub min_net_spread: Decimal,
    /// Maximum position size per trade in USDC
    pub max_position_usdc: Decimal,
    /// Maximum total exposure across all open positions
    pub max_total_exposure_usdc: Decimal,
    /// How often to scan in seconds
    pub scan_interval_secs: u64,
    /// Polymarket CLOB API base URL
    pub polymarket_api: String,
    /// Drift Protocol API base URL
    pub drift_api: String,
    /// Solana RPC endpoint
    pub solana_rpc: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            min_net_spread: Decimal::new(25, 3),       // 2.5%
            max_position_usdc: Decimal::new(500, 0),   // $500 per trade
            max_total_exposure_usdc: Decimal::new(2000, 0), // $2000 total
            scan_interval_secs: 3,
            polymarket_api: "https://clob.polymarket.com".to_string(),
            drift_api: "https://mainnet-beta.api.drift.trade".to_string(),
            solana_rpc: "https://api.mainnet-beta.solana.com".to_string(),
        }
    }
}
