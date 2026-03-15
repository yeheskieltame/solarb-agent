export type Asset = "BTC" | "ETH" | "SOL" | "MATIC" | "ARB";

export type Confidence = "Low" | "Medium" | "High";

export type PositionSide = "Long" | "Short";

export interface ArbOpportunity {
  id: string;
  asset: Asset;
  polymarketProb: number;
  driftProb: number;
  grossSpread: number;
  netSpread: number;
  confidence: Confidence;
  timestamp: number;
}

export interface Position {
  id: string;
  asset: Asset;
  side: PositionSide;
  entryPrice: number;
  currentPrice: number;
  sizeUsdc: number;
  pnl: number;
  pnlPercent: number;
  openedAt: number;
}

export interface AgentStatus {
  isRunning: boolean;
  scanCount: number;
  opportunitiesFound: number;
  tradesExecuted: number;
  totalPnl: number;
  uptime: number;
  lastScan: number;
}

export interface WsMessage {
  type: "opportunity" | "position_update" | "agent_status" | "pnl_update";
  data: ArbOpportunity | Position | AgentStatus | PnlPoint;
}

export interface PnlPoint {
  timestamp: number;
  value: number;
  cumulative: number;
}
