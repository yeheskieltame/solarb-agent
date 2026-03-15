mod detector;
mod scanner;
mod types;

use anyhow::Result;
use dotenvy::dotenv;
use rust_decimal_macros::dec;
use std::env;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use crate::detector::ArbDetector;
use crate::scanner::{DriftScanner, PolymarketScanner};
use crate::types::{AgentConfig, ArbOpportunity, Confidence};

// ── Agent state ───────────────────────────────────────────────────────────────

struct AgentState {
    total_scans: u64,
    opportunities_found: u64,
    high_confidence_found: u64,
    scan_errors: u64,
}

impl AgentState {
    fn new() -> Self {
        Self {
            total_scans: 0,
            opportunities_found: 0,
            high_confidence_found: 0,
            scan_errors: 0,
        }
    }

    fn record_scan(&mut self, opportunities: &[ArbOpportunity]) {
        self.total_scans += 1;
        self.opportunities_found += opportunities.len() as u64;
        self.high_confidence_found +=
            ArbDetector::high_confidence_count(opportunities) as u64;
    }

    fn record_error(&mut self) {
        self.scan_errors += 1;
    }

    fn log_summary(&self) {
        info!(
            "=== Agent summary | scans={} found={} high={} errors={} ===",
            self.total_scans,
            self.opportunities_found,
            self.high_confidence_found,
            self.scan_errors,
        );
    }
}

// ── Main scan loop ────────────────────────────────────────────────────────────

async fn run_scan_cycle(
    poly_scanner: &PolymarketScanner,
    drift_scanner: &DriftScanner,
    detector: &ArbDetector,
    config: &AgentConfig,
) -> Result<Vec<ArbOpportunity>> {
    // Fetch both venues concurrently
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
        "Fetched {} Polymarket signals + {} Drift signals",
        poly_signals.len(),
        drift_signals.len()
    );

    // Run detector
    let opportunities = detector.detect(&poly_signals, &drift_signals);

    // Log results
    if opportunities.is_empty() {
        info!("No actionable opportunities this cycle");
    } else {
        info!("Found {} opportunities:", opportunities.len());
        for (i, opp) in opportunities.iter().enumerate() {
            match opp.confidence {
                Confidence::High => {
                    info!("  [{}] *** HIGH *** {}", i + 1, opp);
                    info!(
                        "       Est. profit on ${}: ${:.2}",
                        config.max_position_usdc,
                        opp.estimated_profit(config.max_position_usdc)
                    );
                    if opp.buy_poly_yes {
                        info!(
                            "       Action: BUY YES on Polymarket (condition: {}) + SHORT {}-PERP on Drift",
                            opp.poly_signal.condition_id,
                            opp.asset,
                        );
                    } else {
                        info!(
                            "       Action: BUY NO on Polymarket (condition: {}) + LONG {}-PERP on Drift",
                            opp.poly_signal.condition_id,
                            opp.asset,
                        );
                    }
                }
                Confidence::Medium => {
                    info!("  [{}] MED {}", i + 1, opp);
                }
                Confidence::Low => {
                    info!("  [{}] low {}", i + 1, opp);
                }
            }
        }
    }

    Ok(opportunities)
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file (optional — will not fail if absent)
    dotenv().ok();

    // Set up tracing/logging
    // RUST_LOG=info cargo run    → normal output
    // RUST_LOG=debug cargo run   → verbose (every calculation)
    // RUST_LOG=trace cargo run   → maximum detail
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .with_level(true)
        .init();

    info!("╔══════════════════════════════════════╗");
    info!("║  SolArb Agent v0.1 — starting up     ║");
    info!("║  Solana Agent Economy Hackathon 2026  ║");
    info!("╚══════════════════════════════════════╝");

    // Build config (override via env vars)
    let config = AgentConfig {
        polymarket_api: env::var("POLYMARKET_API")
            .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
        drift_api: env::var("DRIFT_API")
            .unwrap_or_else(|_| "https://mainnet-beta.api.drift.trade".to_string()),
        solana_rpc: env::var("SOLANA_RPC")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()),
        min_net_spread: env::var("MIN_NET_SPREAD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(dec!(0.025)),
        max_position_usdc: env::var("MAX_POSITION_USDC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(dec!(500)),
        max_total_exposure_usdc: env::var("MAX_TOTAL_EXPOSURE_USDC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(dec!(2000)),
        scan_interval_secs: env::var("SCAN_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3),
    };

    info!("Config:");
    info!("  Polymarket API : {}", config.polymarket_api);
    info!("  Drift API      : {}", config.drift_api);
    info!("  Min net spread : {}%", config.min_net_spread * dec!(100));
    info!("  Max position   : ${}", config.max_position_usdc);
    info!("  Scan interval  : {}s", config.scan_interval_secs);

    // Build components
    let poly_scanner = PolymarketScanner::new(&config.polymarket_api);
    let drift_scanner = DriftScanner::new(&config.drift_api);
    let detector = ArbDetector::new(config.min_net_spread);
    let mut state = AgentState::new();

    info!("Agent ready — scanning every {}s", config.scan_interval_secs);
    info!("Press Ctrl+C to stop\n");

    // Main loop
    loop {
        match run_scan_cycle(&poly_scanner, &drift_scanner, &detector, &config).await {
            Ok(opportunities) => {
                state.record_scan(&opportunities);

                // TODO (Sprint 2): if opportunity is actionable → call executor
                // if let Some(best) = opportunities.first() {
                //     if best.confidence == Confidence::High {
                //         executor.execute(best).await?;
                //     }
                // }
            }
            Err(e) => {
                error!("Scan cycle error: {}", e);
                state.record_error();
            }
        }

        // Every 10 scans, print a summary
        if state.total_scans % 10 == 0 {
            state.log_summary();
        }

        sleep(Duration::from_secs(config.scan_interval_secs)).await;
    }
}
