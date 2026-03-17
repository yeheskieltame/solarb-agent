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
  takeProfit: number;
  stopLoss: number;
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
  mode: string;
}

export interface AiAnalysis {
  summary: string;
  marketSentiment: string;
  topOpportunity: {
    asset: string;
    direction: string;
    aiConfidence: string;
    reasoning: string;
  } | null;
  riskAssessment: string;
  timestamp: number;
}

export interface WsMessage {
  type:
    | "opportunity"
    | "position_update"
    | "agent_status"
    | "pnl_update"
    | "ai_analysis";
  data: ArbOpportunity | Position | AgentStatus | PnlPoint | AiAnalysis;
}

export interface PnlPoint {
  timestamp: number;
  value: number;
  cumulative: number;
}
