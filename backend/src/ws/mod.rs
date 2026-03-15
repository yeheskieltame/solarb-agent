use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::Serialize;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use crate::types::*;

// ── Frontend-compatible DTOs ────────────────────────────────────────────────

/// Wrapper matching frontend WsMessage type
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum WsEvent {
    Opportunity(OpportunityDto),
    PositionUpdate(PositionDto),
    AgentStatus(AgentStatusDto),
    PnlUpdate(PnlPointDto),
}

/// Matches frontend ArbOpportunity
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDto {
    pub id: String,
    pub asset: String,
    pub polymarket_prob: f64,
    pub drift_prob: f64,
    pub gross_spread: f64,
    pub net_spread: f64,
    pub confidence: String,
    pub timestamp: i64,
}

impl OpportunityDto {
    pub fn from_arb(opp: &ArbOpportunity) -> Self {
        Self {
            id: opp.id.clone(),
            asset: opp.asset.to_string(),
            polymarket_prob: dec_to_f64(opp.poly_prob),
            drift_prob: dec_to_f64(opp.drift_prob),
            gross_spread: dec_to_f64(opp.gross_spread),
            net_spread: dec_to_f64(opp.net_spread),
            confidence: opp.confidence.to_string().to_ascii_lowercase(),
            timestamp: opp.detected_at.timestamp_millis(),
        }
    }
}

/// Matches frontend Position
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionDto {
    pub id: String,
    pub asset: String,
    pub side: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub size_usdc: f64,
    pub pnl: f64,
    pub pnl_percent: f64,
    pub opened_at: i64,
}

impl PositionDto {
    pub fn from_position(pos: &Position, current_price: Decimal) -> Self {
        let entry = dec_to_f64(pos.entry_price);
        let current = dec_to_f64(current_price);
        let size = dec_to_f64(pos.size_usdc);

        let pnl = match pos.side {
            PositionSide::Long => (current - entry) / entry * size,
            PositionSide::Short => (entry - current) / entry * size,
        };
        let pnl_percent = if entry > 0.0 {
            match pos.side {
                PositionSide::Long => (current - entry) / entry * 100.0,
                PositionSide::Short => (entry - current) / entry * 100.0,
            }
        } else {
            0.0
        };

        Self {
            id: pos.id.clone(),
            asset: pos.asset.to_string(),
            side: match pos.side {
                PositionSide::Long => "Long".to_string(),
                PositionSide::Short => "Short".to_string(),
            },
            entry_price: entry,
            current_price: current,
            size_usdc: size,
            pnl,
            pnl_percent,
            opened_at: pos.opened_at.timestamp_millis(),
        }
    }
}

/// Matches frontend AgentStatus
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusDto {
    pub is_running: bool,
    pub scan_count: u64,
    pub opportunities_found: u64,
    pub trades_executed: u64,
    pub total_pnl: f64,
    pub uptime: u64,
    pub last_scan: i64,
}

/// Matches frontend PnlPoint
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PnlPointDto {
    pub timestamp: i64,
    pub value: f64,
    pub cumulative: f64,
}

// ── WebSocket Server ────────────────────────────────────────────────────────

pub struct WsServer {
    tx: broadcast::Sender<WsEvent>,
}

impl WsServer {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Returns a broadcast sender for pushing events from the main loop.
    pub fn sender(&self) -> broadcast::Sender<WsEvent> {
        self.tx.clone()
    }

    /// Start listening for WebSocket connections. Runs forever.
    pub async fn run(&self, port: u16) {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("WS server failed to bind on {}: {}", addr, e);
                return;
            }
        };

        info!("WebSocket server listening on ws://{}", addr);

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let rx = self.tx.subscribe();
                    tokio::spawn(handle_client(stream, peer, rx));
                }
                Err(e) => {
                    warn!("WS accept error: {}", e);
                }
            }
        }
    }
}

async fn handle_client(
    stream: TcpStream,
    peer: SocketAddr,
    mut rx: broadcast::Receiver<WsEvent>,
) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            warn!("WS handshake failed from {}: {}", peer, e);
            return;
        }
    };

    info!("WS client connected: {}", peer);

    let (mut sink, mut incoming) = ws.split();

    loop {
        tokio::select! {
            // Forward broadcast events to client
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        let json = match serde_json::to_string(&evt) {
                            Ok(j) => j,
                            Err(_) => continue,
                        };
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WS client {} lagged by {} messages", peer, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle incoming messages (ping/pong, close)
            msg = incoming.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sink.send(Message::Pong(data)).await;
                    }
                    Some(Err(_)) => break,
                    _ => {} // ignore text/binary from client
                }
            }
        }
    }

    info!("WS client disconnected: {}", peer);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn dec_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}
