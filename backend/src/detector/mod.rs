use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, trace};
use crate::scanner::{DriftScanner, PolymarketScanner};
use crate::types::{ArbOpportunity, Asset, Confidence, DriftSignal, PolymarketSignal};

// ── Fee model ─────────────────────────────────────────────────────────────────

struct FeeEstimate {
    poly_fee: Decimal,
    drift_fee: Decimal,
    total: Decimal,
}

fn estimate_fees(poly_signal: &PolymarketSignal) -> FeeEstimate {
    let poly_fee = PolymarketScanner::estimate_taker_fee(poly_signal.yes_mid);
    let drift_fee = DriftScanner::estimate_taker_fee();
    FeeEstimate {
        poly_fee,
        drift_fee,
        total: poly_fee + drift_fee,
    }
}

// ── Spread calculator ─────────────────────────────────────────────────────────

/// The core calculation: given a Polymarket signal and a Drift signal for
/// the same asset and time window, compute the arbitrage spread.
///
/// Returns None if no exploitable spread exists after fees.
/// Calculate spread with a minimum threshold — returns None if below threshold
pub fn calculate_spread(
    poly: &PolymarketSignal,
    drift: &DriftSignal,
    min_net_spread: Decimal,
) -> Option<SpreadResult> {
    let horizon_hours = {
        let mins = (poly.resolves_at - Utc::now()).num_minutes();
        if mins <= 0 {
            return None;
        }
        ((mins as f64 / 60.0).ceil() as u32).max(1)
    };

    let poly_prob = poly.implied_probability();
    let drift_prob = drift.implied_up_probability(horizon_hours);

    let fees = estimate_fees(poly);

    // Determine which side is "cheap" and which is "expensive"
    // poly_prob = P(UP) from Polymarket's perspective
    // drift_prob = P(UP) from Drift's perspective
    //
    // If poly_prob < drift_prob:
    //   → Polymarket underprices UP → buy YES on Poly, short on Drift
    //   → Gross spread = drift_prob - poly_prob
    //
    // If poly_prob > drift_prob:
    //   → Polymarket overprices UP → buy NO on Poly, long on Drift
    //   → Gross spread = poly_prob - drift_prob
    //   → In terms of "UP probability": spread = poly_prob - drift_prob

    let (gross_spread, buy_poly_yes) = if drift_prob > poly_prob {
        (drift_prob - poly_prob, true)
    } else {
        (poly_prob - drift_prob, false)
    };

    let net_spread = gross_spread - fees.total;

    trace!(
        "{} {} | poly_prob={:.3} drift_prob={:.3} gross={:.3} fees={:.3} net={:.3}",
        poly.asset,
        poly.direction,
        poly_prob,
        drift_prob,
        gross_spread,
        fees.total,
        net_spread,
    );

    if net_spread < min_net_spread {
        return None;
    }

    Some(SpreadResult {
        poly_prob,
        drift_prob,
        gross_spread,
        poly_fee: fees.poly_fee,
        drift_fee: fees.drift_fee,
        net_spread,
        buy_poly_yes,
    })
}

pub struct SpreadResult {
    pub poly_prob: Decimal,
    pub drift_prob: Decimal,
    pub gross_spread: Decimal,
    pub poly_fee: Decimal,
    pub drift_fee: Decimal,
    pub net_spread: Decimal,
    pub buy_poly_yes: bool,
}

// ── Confidence scoring ────────────────────────────────────────────────────────

fn score_confidence(net_spread: Decimal) -> Confidence {
    if net_spread >= dec!(0.06) {
        Confidence::High
    } else if net_spread >= dec!(0.035) {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

// ── Detector ─────────────────────────────────────────────────────────────────

pub struct ArbDetector {
    pub min_net_spread: Decimal,
}

impl ArbDetector {
    pub fn new(min_net_spread: Decimal) -> Self {
        Self { min_net_spread }
    }

    /// Cross-match Polymarket signals with Drift signals and return all
    /// actionable arbitrage opportunities, sorted by net spread (best first).
    pub fn detect(
        &self,
        poly_signals: &[PolymarketSignal],
        drift_signals: &[DriftSignal],
    ) -> Vec<ArbOpportunity> {
        let mut opportunities = Vec::new();

        for poly in poly_signals {
            // Find the matching Drift signal for the same asset
            let drift = match drift_signals.iter().find(|d| d.asset == poly.asset) {
                Some(d) => d,
                None => {
                    debug!("No Drift signal for {:?}, skipping", poly.asset);
                    continue;
                }
            };

            // Only compare signals that are freshly captured (within 5s of each other)
            let signal_age_diff = (poly.captured_at - drift.captured_at)
                .num_seconds()
                .abs();
            if signal_age_diff > 5 {
                debug!(
                    "Signal age mismatch for {:?}: {}s — skipping stale pair",
                    poly.asset, signal_age_diff
                );
                continue;
            }

            let mins_to_resolution = (poly.resolves_at - Utc::now()).num_minutes();

            // Need at least 2 minutes to execute both legs safely
            if mins_to_resolution < 2 {
                continue;
            }

            if let Some(spread) = calculate_spread(poly, drift, self.min_net_spread) {
                let confidence = score_confidence(spread.net_spread);
                let id = format!(
                    "{}-{}-{}",
                    poly.asset,
                    poly.direction,
                    poly.captured_at.timestamp()
                );

                let opportunity = ArbOpportunity {
                    id,
                    asset: poly.asset.clone(),
                    direction: poly.direction.clone(),
                    poly_signal: poly.clone(),
                    poly_prob: spread.poly_prob,
                    drift_signal: drift.clone(),
                    drift_prob: spread.drift_prob,
                    gross_spread: spread.gross_spread,
                    poly_fee: spread.poly_fee,
                    drift_fee: spread.drift_fee,
                    net_spread: spread.net_spread,
                    buy_poly_yes: spread.buy_poly_yes,
                    confidence,
                    liquidity_usdc: poly.yes_liquidity,
                    time_to_resolution_mins: mins_to_resolution,
                    detected_at: Utc::now(),
                };

                if opportunity.is_actionable() {
                    opportunities.push(opportunity);
                }
            }
        }

        // Sort by net spread descending — best opportunity first
        opportunities.sort_by(|a, b| b.net_spread.cmp(&a.net_spread));
        opportunities
    }

    /// Return ALL cross-matched pairs regardless of spread threshold.
    /// Used for frontend display — shows monitoring data even when no arbitrage exists.
    pub fn detect_all(
        &self,
        poly_signals: &[PolymarketSignal],
        drift_signals: &[DriftSignal],
    ) -> Vec<ArbOpportunity> {
        let mut all = Vec::new();

        for poly in poly_signals {
            let drift = match drift_signals.iter().find(|d| d.asset == poly.asset) {
                Some(d) => d,
                None => continue,
            };

            let signal_age_diff = (poly.captured_at - drift.captured_at)
                .num_seconds()
                .abs();
            if signal_age_diff > 5 {
                continue;
            }

            let mins_to_resolution = (poly.resolves_at - Utc::now()).num_minutes();
            if mins_to_resolution < 2 {
                continue;
            }

            // Calculate spread with zero threshold — include everything
            if let Some(spread) = calculate_spread(poly, drift, dec!(-1)) {
                let confidence = score_confidence(spread.net_spread);
                let id = format!(
                    "{}-{}-{}",
                    poly.asset,
                    poly.direction,
                    poly.captured_at.timestamp()
                );

                all.push(ArbOpportunity {
                    id,
                    asset: poly.asset.clone(),
                    direction: poly.direction.clone(),
                    poly_signal: poly.clone(),
                    poly_prob: spread.poly_prob,
                    drift_signal: drift.clone(),
                    drift_prob: spread.drift_prob,
                    gross_spread: spread.gross_spread,
                    poly_fee: spread.poly_fee,
                    drift_fee: spread.drift_fee,
                    net_spread: spread.net_spread,
                    buy_poly_yes: spread.buy_poly_yes,
                    confidence,
                    liquidity_usdc: poly.yes_liquidity,
                    time_to_resolution_mins: mins_to_resolution,
                    detected_at: Utc::now(),
                });
            }
        }

        all.sort_by(|a, b| b.net_spread.cmp(&a.net_spread));
        all
    }

    /// How many opportunities are HIGH confidence in the current batch
    pub fn high_confidence_count(opportunities: &[ArbOpportunity]) -> usize {
        opportunities
            .iter()
            .filter(|o| o.confidence == Confidence::High)
            .count()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn make_poly_signal(asset: Asset, yes_mid: Decimal, mins_to_resolve: i64) -> PolymarketSignal {
        use crate::types::Direction;
        PolymarketSignal {
            asset,
            direction: Direction::Up,
            resolves_at: Utc::now() + Duration::minutes(mins_to_resolve),
            yes_bid: yes_mid - dec!(0.01),
            yes_ask: yes_mid + dec!(0.01),
            yes_mid,
            yes_liquidity: dec!(5000),
            condition_id: "test-condition".to_string(),
            yes_token_id: "test-token".to_string(),
            captured_at: Utc::now(),
        }
    }

    fn make_drift_signal(asset: Asset, mark_premium: Decimal, funding: Decimal) -> DriftSignal {
        DriftSignal {
            asset,
            funding_rate_1h: funding,
            mark_price: dec!(65000) * (dec!(1) + mark_premium),
            oracle_price: dec!(65000),
            mark_premium,
            market_index: 1,
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn test_detect_clear_opportunity() {
        let detector = ArbDetector::new(dec!(0.025));

        // Polymarket says BTC UP has 40% probability (bearish/cheap)
        // Drift has 2% mark premium + 0.05%/hr funding (bullish)
        // → Drift implies ~65% UP → spread ~25% gross
        let poly = vec![make_poly_signal(Asset::BTC, dec!(0.40), 15)];
        let drift = vec![make_drift_signal(Asset::BTC, dec!(0.02), dec!(0.0005))];

        let opps = detector.detect(&poly, &drift);
        assert!(!opps.is_empty(), "Should detect opportunity");

        let top = &opps[0];
        assert!(top.net_spread > dec!(0.025));
        assert!(top.buy_poly_yes, "Should buy YES on Poly since Poly underprices UP");
    }

    #[test]
    fn test_no_opportunity_when_prices_aligned() {
        let detector = ArbDetector::new(dec!(0.025));

        // Both venues agree: ~50% probability for BTC UP
        let poly = vec![make_poly_signal(Asset::BTC, dec!(0.50), 15)];
        let drift = vec![make_drift_signal(Asset::BTC, dec!(0), dec!(0))];

        let opps = detector.detect(&poly, &drift);
        assert!(opps.is_empty(), "No opportunity when markets agree");
    }

    #[test]
    fn test_skip_near_expiry() {
        let detector = ArbDetector::new(dec!(0.025));

        // Market expires in 1 minute — too close to safely execute both legs
        let poly = vec![make_poly_signal(Asset::BTC, dec!(0.30), 1)];
        let drift = vec![make_drift_signal(Asset::BTC, dec!(0.03), dec!(0.001))];

        let opps = detector.detect(&poly, &drift);
        assert!(opps.is_empty(), "Should skip near-expiry markets");
    }

    #[test]
    fn test_confidence_levels() {
        assert_eq!(score_confidence(dec!(0.025)), Confidence::Low);
        assert_eq!(score_confidence(dec!(0.04)), Confidence::Medium);
        assert_eq!(score_confidence(dec!(0.07)), Confidence::High);
    }

    #[test]
    fn test_estimated_profit() {
        let detector = ArbDetector::new(dec!(0.025));
        let poly = vec![make_poly_signal(Asset::BTC, dec!(0.35), 20)];
        let drift = vec![make_drift_signal(Asset::BTC, dec!(0.025), dec!(0.0006))];

        let opps = detector.detect(&poly, &drift);
        if let Some(opp) = opps.first() {
            let profit = opp.estimated_profit(dec!(500));
            assert!(profit > dec!(0), "Should be profitable: ${:.2}", profit);
            println!("Estimated profit on $500: ${:.2}", profit);
        }
    }
}
