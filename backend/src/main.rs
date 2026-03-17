#![allow(dead_code)]

mod ai;
mod detector;
mod executor;
mod risk;
mod scanner;
mod types;
mod wallet;
mod ws;

use anyhow::Result;
use chrono::Utc;
use dotenvy::dotenv;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::env;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{sleep, Duration, Instant};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::ai::AiAnalyzer;
use crate::detector::ArbDetector;
use crate::executor::TradeExecutor;
use crate::risk::RiskManager;
use crate::scanner::{DriftScanner, PolymarketScanner};
use crate::types::*;
use crate::wallet::SolWallet;
use crate::ws::{AgentStatusDto, OpportunityDto, PnlPointDto, PositionDto, WsEvent, WsServer};

// ── Agent state ───────────────────────────────────────────────────────────────

struct AgentState {
    total_scans: u64,
    opportunities_found: u64,
    high_confidence_found: u64,
    trades_executed: u64,
    scan_errors: u64,
    started_at: Instant,
}

impl AgentState {
    fn new() -> Self {
        Self {
            total_scans: 0,
            opportunities_found: 0,
            high_confidence_found: 0,
            trades_executed: 0,
            scan_errors: 0,
            started_at: Instant::now(),
        }
    }

    fn record_scan(&mut self, opportunities: &[ArbOpportunity]) {
        self.total_scans += 1;
        self.opportunities_found += opportunities.len() as u64;
        self.high_confidence_found +=
            ArbDetector::high_confidence_count(opportunities) as u64;
    }

    fn record_trade(&mut self) {
        self.trades_executed += 1;
    }

    fn record_error(&mut self) {
        self.scan_errors += 1;
    }

    fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    fn to_status_dto(&self, daily_pnl: Decimal, dry_run: bool) -> AgentStatusDto {
        AgentStatusDto {
            is_running: true,
            scan_count: self.total_scans,
            opportunities_found: self.opportunities_found,
            trades_executed: self.trades_executed,
            total_pnl: dec_to_f64(daily_pnl),
            uptime: self.uptime_secs(),
            last_scan: Utc::now().timestamp_millis(),
            mode: if dry_run { "Dry Run".to_string() } else { "Live".to_string() },
        }
    }

    fn log_summary(&self) {
        info!(
            "=== Agent | scans={} found={} high={} trades={} errors={} ===",
            self.total_scans,
            self.opportunities_found,
            self.high_confidence_found,
            self.trades_executed,
            self.scan_errors,
        );
    }
}

// ── Config loader ────────────────────────────────────────────────────────────

fn load_config() -> AgentConfig {
    let network = match env::var("SOLANA_NETWORK").unwrap_or_default().as_str() {
        "mainnet" => SolanaNetwork::Mainnet,
        _ => SolanaNetwork::Devnet,
    };

    let dry_run = env::var("DRY_RUN")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    AgentConfig {
        polymarket_api: env::var("POLYMARKET_API")
            .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
        drift_api: env::var("DRIFT_API")
            .unwrap_or_else(|_| "https://mainnet-beta.api.drift.trade".to_string()),
        solana_rpc: env::var("SOLANA_RPC")
            .unwrap_or_else(|_| match network {
                SolanaNetwork::Devnet => "https://api.devnet.solana.com".to_string(),
                SolanaNetwork::Mainnet => "https://api.mainnet-beta.solana.com".to_string(),
            }),
        min_net_spread: env_decimal("MIN_NET_SPREAD", dec!(0.025)),
        max_position_usdc: env_decimal("MAX_POSITION_USDC", dec!(500)),
        max_total_exposure_usdc: env_decimal("MAX_TOTAL_EXPOSURE_USDC", dec!(2000)),
        scan_interval_secs: env::var("SCAN_INTERVAL_SECS")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(3),
        network,
        keypair_path: env::var("AGENT_KEYPAIR_PATH").ok(),
        jupiter_api: env::var("JUPITER_API")
            .unwrap_or_else(|_| "https://quote-api.jup.ag/v6".to_string()),
        dry_run,
        daily_loss_stop_usdc: env_decimal("DAILY_LOSS_STOP_USDC", dec!(200)),
        take_profit_pct: env_decimal("TAKE_PROFIT_PCT", dec!(0.03)),  // 3% of entry price
        stop_loss_pct: env_decimal("STOP_LOSS_PCT", dec!(0.05)),    // 5% of entry price
        max_open_positions: env::var("MAX_OPEN_POSITIONS")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(5),
    }
}

fn env_decimal(key: &str, default: Decimal) -> Decimal {
    env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn dec_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}

// ── Main scan loop ────────────────────────────────────────────────────────────

struct ScanResult {
    /// Actionable opportunities (above min spread threshold)
    actionable: Vec<ArbOpportunity>,
    /// All cross-matched signals for display (including below threshold)
    all_signals: Vec<ArbOpportunity>,
}

async fn run_scan_cycle(
    poly_scanner: &PolymarketScanner,
    drift_scanner: &DriftScanner,
    detector: &ArbDetector,
    config: &AgentConfig,
) -> Result<ScanResult> {
    let (poly_result, drift_result) = tokio::join!(
        poly_scanner.fetch_signals(),
        drift_scanner.fetch_all_signals(),
    );

    let poly_signals = poly_result.map_err(|e| {
        warn!("Polymarket fetch error: {}", e);
        e
    })?;

    let drift_signals = drift_result.map_err(|e| {
        warn!("Drift fetch error: {}", e);
        e
    })?;

    info!(
        "Fetched {} Polymarket + {} Drift signals",
        poly_signals.len(),
        drift_signals.len()
    );

    // Get all cross-matched pairs for frontend display
    let all_signals = detector.detect_all(&poly_signals, &drift_signals);

    // Get only actionable opportunities for execution
    let actionable = detector.detect(&poly_signals, &drift_signals);

    if all_signals.is_empty() {
        info!("No cross-matched signals this cycle");
    } else {
        info!("Cross-matched {} signals ({} actionable):", all_signals.len(), actionable.len());
        for (i, opp) in all_signals.iter().enumerate() {
            match opp.confidence {
                Confidence::High => {
                    info!("  [{}] *** HIGH *** {}", i + 1, opp);
                    info!(
                        "       Est. profit on ${}: ${:.2}",
                        config.max_position_usdc,
                        opp.estimated_profit(config.max_position_usdc)
                    );
                }
                Confidence::Medium => info!("  [{}] MED {}", i + 1, opp),
                Confidence::Low => info!("  [{}] low {}", i + 1, opp),
            }
        }
    }

    Ok(ScanResult { actionable, all_signals })
}

// ── Position monitoring ──────────────────────────────────────────────────────

async fn monitor_positions(
    executor: &TradeExecutor,
    risk_manager: &mut RiskManager,
    drift_scanner: &DriftScanner,
    ws_tx: &broadcast::Sender<WsEvent>,
) {
    let open_positions: Vec<Position> = risk_manager.open_positions()
        .into_iter().cloned().collect();

    if open_positions.is_empty() {
        return;
    }

    let drift_signals = match drift_scanner.fetch_all_signals().await {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to fetch Drift signals for monitoring: {}", e);
            return;
        }
    };

    let now = Utc::now();

    for pos in &open_positions {
        let current_price = drift_signals.iter()
            .find(|s| s.asset == pos.asset)
            .map(|s| s.mark_price);

        let current_price = match current_price {
            Some(p) => p,
            None => continue,
        };

        // Broadcast position update to frontend
        let _ = ws_tx.send(WsEvent::PositionUpdate(
            PositionDto::from_position(pos, current_price),
        ));

        // Minimum hold time: 5 minutes before TP exit (SL always allowed)
        let age_mins = (now - pos.opened_at).num_minutes();

        if let Some(reason) = executor.check_exit_conditions(pos, current_price) {
            if reason == ExitReason::TakeProfit && age_mins < 5 {
                info!("TP triggered for {} but position only {}min old — holding", pos.id, age_mins);
                continue;
            }
            info!("Exit triggered for {}: {} (price={:.2})", pos.id, reason, current_price);

            match executor.close_position(pos).await {
                Ok(pnl) => {
                    risk_manager.close_position(&pos.id, pnl);

                    // Broadcast PnL update
                    let _ = ws_tx.send(WsEvent::PnlUpdate(PnlPointDto {
                        timestamp: Utc::now().timestamp_millis(),
                        value: dec_to_f64(pnl),
                        cumulative: dec_to_f64(risk_manager.daily_pnl()),
                    }));
                }
                Err(e) => {
                    warn!("Failed to close position {}: {}", pos.id, e);
                }
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .with_level(true)
        .init();

    let config = load_config();

    info!("SolArb Agent v0.3 starting up");
    info!("Network    : {:?}", config.network);
    info!("Dry run    : {}", config.dry_run);
    info!("Min spread : {}%", config.min_net_spread * dec!(100));
    info!("Max position: ${}", config.max_position_usdc);
    info!("Scan interval: {}s", config.scan_interval_secs);

    // Initialize wallet (optional — dry run works without it)
    let wallet = match &config.keypair_path {
        Some(path) => {
            match SolWallet::from_file(path, &config.solana_rpc, config.network.clone()) {
                Ok(w) => {
                    info!("Wallet     : {}", w.pubkey());
                    w.log_balances().await;
                    Some(Arc::new(w))
                }
                Err(e) => {
                    warn!("Failed to load wallet: {} — running in scan-only mode", e);
                    None
                }
            }
        }
        None => {
            if !config.dry_run {
                warn!("No AGENT_KEYPAIR_PATH set — forcing dry_run=true");
            }
            None
        }
    };

    // Build components
    let poly_scanner = PolymarketScanner::new(&config.polymarket_api);
    let drift_scanner = DriftScanner::new(&config.drift_api);
    let detector = ArbDetector::new(config.min_net_spread);

    let mut risk_manager = RiskManager::new(RiskLimits {
        max_position_usdc: config.max_position_usdc,
        max_total_exposure_usdc: config.max_total_exposure_usdc,
        daily_loss_stop_usdc: config.daily_loss_stop_usdc,
        max_open_positions: config.max_open_positions,
    });

    // Executor requires a wallet — use a dummy keypair for dry run
    let executor_wallet = match wallet {
        Some(ref w) => Arc::clone(w),
        None => {
            let dummy = SolWallet::from_keypair(
                solana_sdk::signature::Keypair::new(),
                &config.solana_rpc,
                config.network.clone(),
            )?;
            Arc::new(dummy)
        }
    };

    let executor = TradeExecutor::new(Arc::clone(&executor_wallet), &config);

    // Check gateway connectivity for live mode
    if !config.dry_run {
        if !executor.check_gateway().await {
            warn!("Drift Gateway not reachable — falling back to dry-run mode");
            warn!("To enable live trading, run the Drift Gateway: https://github.com/drift-labs/gateway");
        }
    }

    let mut state = AgentState::new();

    // Start WebSocket server
    let ws_port: u16 = env::var("WS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9944);

    let ws_server = WsServer::new(256);
    let ws_tx = ws_server.sender();

    tokio::spawn(async move {
        ws_server.run(ws_port).await;
    });

    // Initialize AI analyzer — supports multiple providers via AI_PROVIDER env var
    // Providers: "claude-cli" (Max subscription via CLI), "claude" (API key), "gemini"
    let ai_provider = env::var("AI_PROVIDER").unwrap_or_default().to_lowercase();
    let ai_analyzer: Option<AiAnalyzer> = match ai_provider.as_str() {
        "claude-cli" | "cli" => {
            // Uses authenticated `claude` CLI binary — works with Claude Max subscription
            let model = env::var("CLAUDE_MODEL").ok();
            let analyzer = AiAnalyzer::claude_cli(model.as_deref());
            info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
            Some(analyzer)
        }
        "claude" | "anthropic" => {
            match env::var("ANTHROPIC_API_KEY") {
                Ok(key) => {
                    // Auto-detect: OAuth token (sk-ant-oat*) → suggest CLI mode
                    if key.starts_with("sk-ant-oat") {
                        warn!("Detected OAuth token (Claude Max) — switching to claude-cli mode");
                        warn!("OAuth tokens don't work with the API directly. Using `claude` CLI instead.");
                        let model = env::var("CLAUDE_MODEL").ok();
                        let analyzer = AiAnalyzer::claude_cli(model.as_deref());
                        info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
                        Some(analyzer)
                    } else {
                        let model = env::var("CLAUDE_MODEL").ok();
                        let analyzer = AiAnalyzer::claude(&key, model.as_deref());
                        info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
                        Some(analyzer)
                    }
                }
                Err(_) => {
                    // No API key — try CLI mode as fallback
                    warn!("No ANTHROPIC_API_KEY set — trying claude CLI mode");
                    let model = env::var("CLAUDE_MODEL").ok();
                    let analyzer = AiAnalyzer::claude_cli(model.as_deref());
                    info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
                    Some(analyzer)
                }
            }
        }
        "gemini" | "google" => {
            match env::var("GEMINI_API_KEY") {
                Ok(key) => {
                    let analyzer = AiAnalyzer::gemini(&key);
                    info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
                    Some(analyzer)
                }
                Err(_) => {
                    warn!("AI_PROVIDER=gemini but GEMINI_API_KEY not set — AI disabled");
                    None
                }
            }
        }
        _ => {
            // Auto-detect: try Claude API key, then OAuth→CLI, then Gemini, then CLI
            if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
                let model = env::var("CLAUDE_MODEL").ok();
                if key.starts_with("sk-ant-oat") {
                    info!("Detected OAuth token — using claude CLI mode");
                    let analyzer = AiAnalyzer::claude_cli(model.as_deref());
                    info!("AI provider : {} ({}) [auto-detected]", analyzer.provider_name(), analyzer.model_name());
                    Some(analyzer)
                } else {
                    let analyzer = AiAnalyzer::claude(&key, model.as_deref());
                    info!("AI provider : {} ({}) [auto-detected]", analyzer.provider_name(), analyzer.model_name());
                    Some(analyzer)
                }
            } else if let Ok(key) = env::var("GEMINI_API_KEY") {
                let analyzer = AiAnalyzer::gemini(&key);
                info!("AI provider : {} ({}) [auto-detected]", analyzer.provider_name(), analyzer.model_name());
                Some(analyzer)
            } else {
                info!("No AI API key set — trying claude CLI as fallback");
                let model = env::var("CLAUDE_MODEL").ok();
                let analyzer = AiAnalyzer::claude_cli(model.as_deref());
                info!("AI provider : {} ({})", analyzer.provider_name(), analyzer.model_name());
                Some(analyzer)
            }
        }
    };

    // Test AI connectivity at startup
    if let Some(ref analyzer) = ai_analyzer {
        match analyzer.test_connection().await {
            Ok(()) => info!("AI connectivity: OK"),
            Err(e) => warn!("AI connectivity: FAILED — {}", e),
        }
    }

    info!("Agent ready — scanning every {}s", config.scan_interval_secs);
    info!("Press Ctrl+C to stop\n");

    loop {
        // 1. Scan for opportunities
        match run_scan_cycle(&poly_scanner, &drift_scanner, &detector, &config).await {
            Ok(scan_result) => {
                state.record_scan(&scan_result.actionable);

                // Broadcast ALL cross-matched signals to frontend (not just actionable)
                for opp in &scan_result.all_signals {
                    let _ = ws_tx.send(WsEvent::Opportunity(
                        OpportunityDto::from_arb(opp),
                    ));
                }

                // 2. AI-driven strategy or fallback execution
                if let Some(ref analyzer) = ai_analyzer {
                    // Ask AI for strategy (respects 2min cooldown internally)
                    let open_pos: Vec<Position> = risk_manager.open_positions()
                        .into_iter().cloned().collect();
                    let exposure = risk_manager.total_open_exposure();

                    match analyzer.get_strategy(
                        &scan_result.all_signals,
                        &open_pos,
                        exposure,
                        config.max_total_exposure_usdc,
                    ).await {
                        Ok(decision) => {
                            // Broadcast AI analysis to frontend
                            let _ = ws_tx.send(WsEvent::AiAnalysis(decision.analysis));

                            // Close positions AI recommends closing (with min hold time guard)
                            let now_ts = Utc::now();
                            for close_id in &decision.close {
                                if let Some(pos) = open_pos.iter().find(|p| &p.id == close_id) {
                                    let age_mins = (now_ts - pos.opened_at).num_minutes();
                                    if age_mins < 5 {
                                        info!("AI wants to close {} but position only {}min old — skipping", &close_id[..8.min(close_id.len())], age_mins);
                                        continue;
                                    }
                                    match executor.close_position(pos).await {
                                        Ok(pnl) => {
                                            risk_manager.close_position(close_id, pnl);
                                            let _ = ws_tx.send(WsEvent::PnlUpdate(PnlPointDto {
                                                timestamp: Utc::now().timestamp_millis(),
                                                value: dec_to_f64(pnl),
                                                cumulative: dec_to_f64(risk_manager.daily_pnl()),
                                            }));
                                            info!("AI closed position {} | PnL: ${:.2}", &close_id[..8], pnl);
                                        }
                                        Err(e) => warn!("Failed to close {}: {}", &close_id[..8], e),
                                    }
                                }
                            }

                            // Execute opportunities AI recommends
                            for &idx in &decision.execute {
                                if let Some(opp) = scan_result.all_signals.get(idx) {
                                    let size = risk_manager.size_for_opportunity(
                                        config.max_position_usdc,
                                        opp.liquidity_usdc,
                                        if config.dry_run { config.max_position_usdc }
                                        else { executor_wallet.usdc_balance().await.unwrap_or(dec!(0)) },
                                    );

                                    if size > dec!(10) {
                                        match risk_manager.can_open(size, &opp.asset) {
                                            Ok(()) => {
                                                match executor.execute_opportunity(opp, size).await {
                                                    Ok(position) => {
                                                        let _ = ws_tx.send(WsEvent::PositionUpdate(
                                                            PositionDto::from_position(&position, position.entry_price),
                                                        ));
                                                        risk_manager.open_position(position);
                                                        state.record_trade();
                                                    }
                                                    Err(e) => warn!("Execution failed: {}", e),
                                                }
                                            }
                                            Err(denial) => info!("Risk denied: {}", denial),
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // Log the actual error (not just silent swallow)
                            let err_msg = format!("{}", e);
                            if !err_msg.contains("cooldown") {
                                warn!("AI strategy call failed: {}", e);
                            }
                            // AI on cooldown or failed — use fallback logic
                            let opportunities = &scan_result.actionable;
                            for best in opportunities.iter().filter(|o| o.confidence == Confidence::High) {
                                let size = risk_manager.size_for_opportunity(
                                    config.max_position_usdc,
                                    best.liquidity_usdc,
                                    if config.dry_run { config.max_position_usdc }
                                    else { executor_wallet.usdc_balance().await.unwrap_or(dec!(0)) },
                                );

                                if size > dec!(10) {
                                    if let Ok(()) = risk_manager.can_open(size, &best.asset) {
                                        match executor.execute_opportunity(best, size).await {
                                            Ok(position) => {
                                                let _ = ws_tx.send(WsEvent::PositionUpdate(
                                                    PositionDto::from_position(&position, position.entry_price),
                                                ));
                                                risk_manager.open_position(position);
                                                state.record_trade();
                                            }
                                            Err(e) => warn!("Fallback execution failed: {}", e),
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // No AI key — fallback: execute best single high-confidence opp
                    let opportunities = &scan_result.actionable;
                    if let Some(best) = opportunities.iter().find(|o| o.confidence == Confidence::High) {
                        let size = risk_manager.size_for_opportunity(
                            config.max_position_usdc,
                            best.liquidity_usdc,
                            if config.dry_run { config.max_position_usdc }
                            else { executor_wallet.usdc_balance().await.unwrap_or(dec!(0)) },
                        );

                        if size > dec!(10) {
                            if let Ok(()) = risk_manager.can_open(size, &best.asset) {
                                match executor.execute_opportunity(best, size).await {
                                    Ok(position) => {
                                        let _ = ws_tx.send(WsEvent::PositionUpdate(
                                            PositionDto::from_position(&position, position.entry_price),
                                        ));
                                        risk_manager.open_position(position);
                                        state.record_trade();
                                    }
                                    Err(e) => warn!("Execution failed: {}", e),
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Scan cycle error: {}", e);
                state.record_error();
            }
        }

        // 3. Monitor open positions for exit conditions
        monitor_positions(&executor, &mut risk_manager, &drift_scanner, &ws_tx).await;

        // 4. Broadcast agent status every cycle
        let _ = ws_tx.send(WsEvent::AgentStatus(
            state.to_status_dto(risk_manager.daily_pnl(), config.dry_run),
        ));

        // 5. Periodic summary
        if state.total_scans % 10 == 0 {
            state.log_summary();
            risk_manager.log_summary();
        }

        sleep(Duration::from_secs(config.scan_interval_secs)).await;
    }
}
