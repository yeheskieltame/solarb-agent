"use client";

import type { ArbOpportunity } from "@/lib/types";

interface Props {
  opportunities: ArbOpportunity[];
}

export default function LiveFeed({ opportunities }: Props) {
  return (
    <div className="glass-card-static p-6">
      <h2 className="text-lg font-semibold text-text-primary mb-4">
        Live Opportunities
      </h2>

      {opportunities.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-text-muted">
          <div className="h-3 w-3 rounded-full bg-accent-cyan/30 animate-ping mb-4" />
          <p className="text-sm">Scanning for arbitrage...</p>
        </div>
      ) : (
        <div className="space-y-3 max-h-[400px] overflow-y-auto">
          {opportunities.map((opp) => (
            <OpportunityRow key={opp.id} opportunity={opp} />
          ))}
        </div>
      )}
    </div>
  );
}

function OpportunityRow({ opportunity }: { opportunity: ArbOpportunity }) {
  const confidenceColor =
    opportunity.confidence === "High"
      ? "text-profit bg-profit/10 border-profit/20"
      : opportunity.confidence === "Medium"
        ? "text-warning bg-warning/10 border-warning/20"
        : "text-text-muted bg-text-muted/10 border-text-muted/20";

  const timeStr = new Date(opportunity.timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

  return (
    <div className="flex items-center gap-4 rounded-2xl bg-bg-surface/50 px-4 py-3 border border-border-glass transition-colors hover:border-border-glow">
      {/* Asset */}
      <div className="min-w-[60px]">
        <p className="text-sm font-semibold text-text-primary">
          {opportunity.asset}
        </p>
        <p className="text-xs text-text-muted">{timeStr}</p>
      </div>

      {/* Probabilities */}
      <div className="flex-1 grid grid-cols-2 gap-2 text-center">
        <div>
          <p className="text-xs text-text-muted">Polymarket</p>
          <p className="text-sm font-tabular text-accent-violet">
            {(opportunity.polymarketProb * 100).toFixed(1)}%
          </p>
        </div>
        <div>
          <p className="text-xs text-text-muted">Drift</p>
          <p className="text-sm font-tabular text-accent-cyan">
            {(opportunity.driftProb * 100).toFixed(1)}%
          </p>
        </div>
      </div>

      {/* Spread */}
      <div className="text-right min-w-[70px]">
        <p className="text-sm font-bold font-tabular text-profit">
          {(opportunity.netSpread * 100).toFixed(2)}%
        </p>
        <p className="text-xs text-text-muted">net spread</p>
      </div>

      {/* Confidence badge */}
      <span
        className={`rounded-lg border px-2 py-0.5 text-xs font-medium ${confidenceColor}`}
      >
        {opportunity.confidence}
      </span>
    </div>
  );
}
