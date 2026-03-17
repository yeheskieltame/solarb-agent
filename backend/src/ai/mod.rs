use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::types::{ArbOpportunity, Position};

// ── Gemini API types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    generation_config: GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    max_output_tokens: u32,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiCandidateContent,
}

#[derive(Deserialize)]
struct GeminiCandidateContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: String,
}

// ── Claude (Anthropic) API types ─────────────────────────────────────────────

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    temperature: f32,
    messages: Vec<ClaudeMessage>,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Option<Vec<ClaudeContentBlock>>,
    error: Option<ClaudeError>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeError {
    message: String,
}

// ── AI Strategy Decision ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAnalysis {
    pub summary: String,
    pub market_sentiment: String,
    pub top_opportunity: Option<AiOpportunityInsight>,
    pub risk_assessment: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOpportunityInsight {
    pub asset: String,
    pub direction: String,
    pub ai_confidence: String,
    pub reasoning: String,
}

/// AI decision: which opportunities to execute and which positions to close
#[derive(Debug, Clone)]
pub struct AiStrategyDecision {
    /// Indices of opportunities to execute (from the all_signals list)
    pub execute: Vec<usize>,
    /// Position IDs to close
    pub close: Vec<String>,
    /// Human-readable reasoning
    pub reasoning: String,
    /// Frontend-facing analysis
    pub analysis: AiAnalysis,
}

// ── Provider selection ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiProviderType {
    Gemini,
    Claude,
    ClaudeCli,
}

impl std::fmt::Display for AiProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProviderType::Gemini => write!(f, "Gemini"),
            AiProviderType::Claude => write!(f, "Claude API"),
            AiProviderType::ClaudeCli => write!(f, "Claude CLI"),
        }
    }
}

// ── Unified AI Analyzer ─────────────────────────────────────────────────────

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const GEMINI_MODEL: &str = "gemini-2.5-flash";

const CLAUDE_API_BASE: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_API_VERSION: &str = "2023-06-01";

pub struct AiAnalyzer {
    provider: AiProviderType,
    client: Client,
    api_key: String,
    model: String,
    /// Minimum seconds between API calls
    cooldown_secs: u64,
    last_call: std::sync::Mutex<std::time::Instant>,
}

impl AiAnalyzer {
    /// Create a Gemini-backed analyzer
    pub fn gemini(api_key: &str) -> Self {
        Self {
            provider: AiProviderType::Gemini,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            api_key: api_key.to_string(),
            model: GEMINI_MODEL.to_string(),
            cooldown_secs: 20,
            last_call: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(20),
            ),
        }
    }

    /// Create a Claude-backed analyzer
    /// `model` defaults to "claude-sonnet-4-6" if None
    pub fn claude(api_key: &str, model: Option<&str>) -> Self {
        let model = model.unwrap_or("claude-sonnet-4-6").to_string();
        Self {
            provider: AiProviderType::Claude,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("failed to build HTTP client"),
            api_key: api_key.to_string(),
            model,
            cooldown_secs: 20,
            last_call: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(20),
            ),
        }
    }

    /// Create a Claude CLI-backed analyzer (uses local `claude` binary with Max subscription auth)
    /// No API key needed — uses the authenticated Claude Code CLI
    pub fn claude_cli(model: Option<&str>) -> Self {
        let model = model.unwrap_or("claude-sonnet-4-6").to_string();
        Self {
            provider: AiProviderType::ClaudeCli,
            client: Client::new(), // unused for CLI mode
            api_key: String::new(),
            model,
            cooldown_secs: 20,
            last_call: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(20),
            ),
        }
    }

    pub fn provider_name(&self) -> &str {
        match self.provider {
            AiProviderType::Gemini => "Gemini",
            AiProviderType::Claude => "Claude API",
            AiProviderType::ClaudeCli => "Claude CLI",
        }
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Quick connectivity test — sends a minimal prompt to verify API key works
    pub async fn test_connection(&self) -> Result<()> {
        info!("Testing {} API connection...", self.provider);
        let test_prompt = "Respond with exactly: OK";
        let result = match self.provider {
            AiProviderType::Gemini => self.call_gemini(test_prompt).await,
            AiProviderType::Claude => self.call_claude(test_prompt).await,
            AiProviderType::ClaudeCli => self.call_claude_cli(test_prompt).await,
        };
        match result {
            Ok(resp) => {
                info!("{} API test successful: {}", self.provider, &resp[..resp.len().min(50)]);
                Ok(())
            }
            Err(e) => {
                anyhow::bail!("{} API test FAILED: {}", self.provider, e);
            }
        }
    }

    fn can_call(&self) -> bool {
        let last = self.last_call.lock().unwrap();
        last.elapsed().as_secs() >= self.cooldown_secs
    }

    fn mark_called(&self) {
        let mut last = self.last_call.lock().unwrap();
        *last = std::time::Instant::now();
    }

    /// Ask AI for strategy: which opportunities to take, which positions to close.
    pub async fn get_strategy(
        &self,
        opportunities: &[ArbOpportunity],
        open_positions: &[Position],
        total_exposure: Decimal,
        max_exposure: Decimal,
    ) -> Result<AiStrategyDecision> {
        if !self.can_call() {
            anyhow::bail!("AI cooldown active");
        }

        let prompt = build_strategy_prompt(opportunities, open_positions, total_exposure, max_exposure);

        let raw = match self.provider {
            AiProviderType::Gemini => self.call_gemini(&prompt).await?,
            AiProviderType::Claude => self.call_claude(&prompt).await?,
            AiProviderType::ClaudeCli => self.call_claude_cli(&prompt).await?,
        };
        self.mark_called();

        debug!("{} strategy response: {}", self.provider, &raw[..raw.len().min(500)]);

        Ok(parse_strategy(&raw, opportunities, open_positions))
    }

    // ── Gemini HTTP call ─────────────────────────────────────────────────────

    async fn call_gemini(&self, prompt: &str) -> Result<String> {
        let url = format!(
            "{}/{}:generateContent?key={}",
            GEMINI_API_BASE, self.model, self.api_key
        );

        let request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![GeminiPart { text: prompt.to_string() }],
            }],
            generation_config: GeminiGenerationConfig {
                temperature: 0.2,
                max_output_tokens: 400,
            },
        };

        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Gemini API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API returned {}: {}", status, body);
        }

        let gemini_resp: GeminiResponse = resp
            .json()
            .await
            .context("Failed to parse Gemini response")?;

        Ok(gemini_resp
            .candidates
            .and_then(|c| c.into_iter().next())
            .map(|c| {
                c.content
                    .parts
                    .into_iter()
                    .map(|p| p.text)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default())
    }

    // ── Claude HTTP call ─────────────────────────────────────────────────────

    async fn call_claude(&self, prompt: &str) -> Result<String> {
        let request = ClaudeRequest {
            model: self.model.clone(),
            max_tokens: 400,
            temperature: 0.2,
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
        };

        // Auto-detect auth method:
        // - sk-ant-oat* (OAuth Access Token) → Bearer auth (Claude Max subscription)
        // - sk-ant-api* (API key) → x-api-key header (pay-as-you-go)
        let mut req_builder = self
            .client
            .post(CLAUDE_API_BASE)
            .header("anthropic-version", CLAUDE_API_VERSION)
            .header("content-type", "application/json");

        if self.api_key.starts_with("sk-ant-oat") {
            // OAuth token from Claude Max subscription
            req_builder = req_builder.header("Authorization", format!("Bearer {}", self.api_key));
        } else {
            // Standard API key
            req_builder = req_builder.header("x-api-key", &self.api_key);
        }

        let resp = req_builder
            .json(&request)
            .send()
            .await
            .context("Claude API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Claude API returned {}: {}", status, body);
        }

        let claude_resp: ClaudeResponse = resp
            .json()
            .await
            .context("Failed to parse Claude response")?;

        if let Some(err) = claude_resp.error {
            anyhow::bail!("Claude API error: {}", err.message);
        }

        Ok(claude_resp
            .content
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join(""))
    }

    // ── Claude CLI call (uses authenticated `claude` binary) ─────────────
    // Pipes prompt via stdin — passing as CLI arg hangs in subprocess context.
    // Uses Max subscription quota, no API key needed.

    async fn call_claude_cli(&self, prompt: &str) -> Result<String> {
        use tokio::process::Command;
        use std::process::Stdio;

        let claude_bin = find_claude_binary();

        let mut child = Command::new(&claude_bin)
            .args([
                "--print",
                "--model", &self.model,
                "--max-turns", "1",
            ])
            // Clear ANTHROPIC_API_KEY so CLI uses its own Max subscription auth
            // (OAuth tokens in env confuse the CLI into thinking it's an API key)
            .env("ANTHROPIC_API_KEY", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(format!("Failed to spawn `{}` — is Claude Code installed?", claude_bin))?;

        // Write prompt via stdin (avoids shell escaping and arg-length issues)
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(prompt.as_bytes()).await
                .context("Failed to write prompt to claude stdin")?;
            // Drop stdin to signal EOF
            drop(stdin);
        }

        let output = child.wait_with_output().await
            .context("Failed to read claude CLI output")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { &stderr } else { &stdout };
            anyhow::bail!("claude CLI exited {}: {}", output.status, detail);
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            anyhow::bail!("claude CLI returned empty response");
        }

        Ok(stdout)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Find the `claude` CLI binary — checks common install locations
fn find_claude_binary() -> String {
    // Check well-known paths first
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.local/bin/claude", home),
        format!("{}/.claude/bin/claude", home),
        "/usr/local/bin/claude".to_string(),
    ];

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return path.clone();
        }
    }

    // Fallback: hope it's in PATH
    "claude".to_string()
}

// ── Shared prompt builder ───────────────────────────────────────────────────

fn build_strategy_prompt(
    opportunities: &[ArbOpportunity],
    open_positions: &[Position],
    total_exposure: Decimal,
    max_exposure: Decimal,
) -> String {
    let now = chrono::Utc::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut p = format!(
        "You are SolArb Agent — an autonomous cross-venue arbitrage strategist.\n\
         Timestamp: {}\n\n\
         YOUR MANDATE:\n\
         You have FULL authority over all trade decisions. Your decisions are executed immediately.\n\
         Maximize portfolio PnL through intelligent position management.\n\n\
         STRATEGY RULES:\n\
         1. EXECUTE multiple opportunities — fill up to max exposure, each position costs $500\n\
         2. Never hold BOTH Long AND Short on the same asset — they cancel out\n\
         3. Pick the BEST direction per asset (larger net spread wins)\n\
         4. Close conflicting positions to free budget for better opportunities\n\
         5. Close losing positions early to cut losses\n\
         6. Prefer higher net spread + higher liquidity\n\
         7. Only execute if net spread > 5%\n\
         8. You may close ANY position at ANY time if you judge it optimal\n\n",
        timestamp
    );

    // Portfolio state
    let remaining = max_exposure - total_exposure;
    let open_count = open_positions.len();
    p.push_str(&format!(
        "PORTFOLIO STATE:\n\
         - Exposure: ${:.0} / ${:.0} USDC (${:.0} available)\n\
         - Open positions: {} / 5 slots\n\n",
        dec_to_f64(total_exposure),
        dec_to_f64(max_exposure),
        dec_to_f64(remaining),
        open_count,
    ));

    // Open positions with detailed info
    if !open_positions.is_empty() {
        p.push_str("OPEN POSITIONS:\n");
        for pos in open_positions {
            let age_secs = (now - pos.opened_at).num_seconds();
            let age_str = if age_secs < 60 {
                format!("{}s", age_secs)
            } else {
                format!("{}m{}s", age_secs / 60, age_secs % 60)
            };
            p.push_str(&format!(
                "  [{id}] {asset} {side} ${size:.0} @ {entry:.2} | TP={tp:.2} SL={sl:.2} | opened={age} ago\n",
                id = &pos.id[..8],
                asset = pos.asset,
                side = pos.side,
                size = dec_to_f64(pos.size_usdc),
                entry = dec_to_f64(pos.entry_price),
                tp = dec_to_f64(pos.take_profit_price),
                sl = dec_to_f64(pos.stop_loss_price),
                age = age_str,
            ));
        }
        p.push('\n');
    } else {
        p.push_str("OPEN POSITIONS: none\n\n");
    }

    // Opportunities with clear indexing
    p.push_str(&format!("SCAN RESULTS ({} signals):\n", opportunities.len()));
    for (i, opp) in opportunities.iter().enumerate() {
        let est_profit = dec_to_f64(opp.net_spread) * 500.0;
        p.push_str(&format!(
            "  [{i}] {asset} {dir} | Poly={poly:.1}% Drift={drift:.1}% | spread={net:.2}% | est_profit=${profit:.0} | liq=${liq:.0} | resolves_in={mins}min\n",
            i = i,
            asset = opp.asset,
            dir = opp.direction,
            poly = dec_to_f64(opp.poly_prob) * 100.0,
            drift = dec_to_f64(opp.drift_prob) * 100.0,
            net = dec_to_f64(opp.net_spread) * 100.0,
            profit = est_profit,
            liq = dec_to_f64(opp.liquidity_usdc),
            mins = opp.time_to_resolution_mins,
        ));
    }

    p.push_str(
        "\nRespond in EXACTLY this format (one per line, no extra text):\n\
         EXECUTE|comma-separated indices (e.g. 0,2,4) or NONE\n\
         CLOSE|comma-separated position ID prefixes (e.g. abc12345,def67890) or NONE\n\
         SUMMARY|One sentence market overview\n\
         SENTIMENT|Bullish or Bearish or Neutral\n\
         REASONING|Why these trades\n\
         RISK|One sentence risk note\n"
    );

    p
}

// ── Shared response parser ──────────────────────────────────────────────────

fn parse_strategy(
    raw: &str,
    opportunities: &[ArbOpportunity],
    open_positions: &[Position],
) -> AiStrategyDecision {
    let lines: Vec<&str> = raw.lines().collect();

    let get_field = |prefix: &str| -> String {
        lines
            .iter()
            .find(|l| l.starts_with(prefix))
            .map(|l| l[prefix.len()..].trim().to_string())
            .unwrap_or_default()
    };

    // Parse EXECUTE indices
    let execute_str = get_field("EXECUTE|");
    let execute: Vec<usize> = if execute_str == "NONE" || execute_str.is_empty() {
        vec![]
    } else {
        execute_str
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|&i| i < opportunities.len())
            .collect()
    };

    // Parse CLOSE position IDs
    let close_str = get_field("CLOSE|");
    let close: Vec<String> = if close_str == "NONE" || close_str.is_empty() {
        vec![]
    } else {
        let prefixes: Vec<&str> = close_str.split(',').map(|s| s.trim()).collect();
        open_positions
            .iter()
            .filter(|p| prefixes.iter().any(|prefix| p.id.starts_with(prefix)))
            .map(|p| p.id.clone())
            .collect()
    };

    let summary = get_field("SUMMARY|");
    let sentiment = get_field("SENTIMENT|");
    let reasoning = get_field("REASONING|");
    let risk = get_field("RISK|");

    // Build top opportunity insight from the first execute index
    let top_opportunity = execute.first().and_then(|&i| {
        opportunities.get(i).map(|opp| AiOpportunityInsight {
            asset: opp.asset.to_string(),
            direction: opp.direction.to_string(),
            ai_confidence: format!("{}", opp.confidence),
            reasoning: reasoning.clone(),
        })
    });

    info!(
        "AI Strategy: execute={:?} close={:?} | {}",
        execute,
        close
            .iter()
            .map(|id| &id[..8.min(id.len())])
            .collect::<Vec<_>>(),
        if reasoning.is_empty() {
            &summary
        } else {
            &reasoning
        }
    );

    AiStrategyDecision {
        execute,
        close,
        reasoning: reasoning.clone(),
        analysis: AiAnalysis {
            summary: if summary.is_empty() {
                reasoning
            } else {
                summary
            },
            market_sentiment: if sentiment.is_empty() {
                "Neutral".to_string()
            } else {
                sentiment
            },
            top_opportunity,
            risk_assessment: if risk.is_empty() {
                "Monitor positions.".to_string()
            } else {
                risk
            },
            timestamp: chrono::Utc::now().timestamp_millis(),
        },
    }
}

fn dec_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}
