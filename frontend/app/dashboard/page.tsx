"use client";

import { useState, useCallback } from "react";
import Image from "next/image";
import Link from "next/link";
import AnimatedBg from "@/components/AnimatedBg";
import AgentStats from "@/components/AgentStats";
import AiInsights from "@/components/AiInsights";
import LiveFeed from "@/components/LiveFeed";
import PositionCard from "@/components/PositionCard";
import PnlChart from "@/components/PnlChart";
import WalletConnect from "@/components/WalletConnect";
import { useWebSocket } from "@/hooks/useWebSocket";
import type {
  AgentStatus,
  AiAnalysis,
  ArbOpportunity,
  Position,
  PnlPoint,
  WsMessage,
} from "@/lib/types";

const MAX_FEED_ITEMS = 50;

const defaultStatus: AgentStatus = {
  isRunning: false,
  scanCount: 0,
  opportunitiesFound: 0,
  tradesExecuted: 0,
  totalPnl: 0,
  uptime: 0,
  lastScan: Date.now(),
  mode: "Dry Run",
};

export default function Dashboard() {
  const [status, setStatus] = useState<AgentStatus>(defaultStatus);
  const [opportunities, setOpportunities] = useState<ArbOpportunity[]>([]);
  const [positions, setPositions] = useState<Position[]>([]);
  const [pnlHistory, setPnlHistory] = useState<PnlPoint[]>([]);
  const [aiAnalysis, setAiAnalysis] = useState<AiAnalysis | null>(null);

  const handleMessage = useCallback((msg: WsMessage) => {
    switch (msg.type) {
      case "opportunity": {
        const opp = msg.data as ArbOpportunity;
        setOpportunities((prev) => {
          const exists = prev.some((p) => p.id === opp.id);
          if (exists) {
            return prev.map((p) => (p.id === opp.id ? opp : p));
          }
          return [opp, ...prev].slice(0, MAX_FEED_ITEMS);
        });
        break;
      }
      case "position_update":
        setPositions((prev) => {
          const updated = msg.data as Position;
          const idx = prev.findIndex((p) => p.id === updated.id);
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = updated;
            return next;
          }
          return [updated, ...prev];
        });
        break;
      case "agent_status":
        setStatus(msg.data as AgentStatus);
        break;
      case "pnl_update":
        setPnlHistory((prev) => [...prev, msg.data as PnlPoint]);
        break;
      case "ai_analysis":
        setAiAnalysis(msg.data as AiAnalysis);
        break;
    }
  }, []);

  const { connected } = useWebSocket(handleMessage);

  return (
    <>
      <AnimatedBg variant="dashboard" />

      <div className="relative z-10 min-h-screen">
        {/* Header */}
        <header className="glass-card-static flex items-center justify-between px-6 py-3 mx-4 mt-4 sm:mx-6">
          <div className="flex items-center gap-3">
            <div className="relative h-8 w-8">
              <Image
                src="/bg/agent-character.webp"
                alt="SolArb"
                fill
                className="object-contain"
                sizes="32px"
              />
            </div>
            <Link
              href="/"
              className="text-lg font-bold text-text-primary hover:text-accent-cyan transition-colors"
            >
              SolArb Agent
            </Link>
            <ConnectionBadge connected={connected} />
          </div>
          <WalletConnect />
        </header>

        {/* Content */}
        <main className="max-w-7xl mx-auto px-4 sm:px-6 py-6 space-y-6">
          <AgentStats status={status} />

          <div className="grid gap-6 lg:grid-cols-2">
            <div className="space-y-6">
              <LiveFeed opportunities={opportunities} />
            </div>
            <div className="space-y-6">
              <AiInsights analysis={aiAnalysis} />
              <PnlChart data={pnlHistory} />
              <PositionCard positions={positions} />
            </div>
          </div>
        </main>
      </div>
    </>
  );
}

function ConnectionBadge({ connected }: { connected: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-lg px-2 py-0.5 text-xs font-medium ${
        connected
          ? "bg-profit/10 text-profit"
          : "bg-loss/10 text-loss"
      }`}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full ${
          connected ? "bg-profit animate-pulse" : "bg-loss"
        }`}
      />
      {connected ? "Live" : "Offline"}
    </span>
  );
}
