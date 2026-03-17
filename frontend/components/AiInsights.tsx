"use client";

import type { AiAnalysis } from "@/lib/types";

interface Props {
  analysis: AiAnalysis | null;
}

export default function AiInsights({ analysis }: Props) {
  if (!analysis) {
    return (
      <div className="glass-card-static p-6">
        <h2 className="text-lg font-semibold text-text-primary mb-4">
          AI Analysis
        </h2>
        <div className="flex flex-col items-center justify-center py-8 text-text-muted">
          <div className="h-3 w-3 rounded-full bg-accent-violet/30 animate-ping mb-4" />
          <p className="text-sm">Waiting for AI analysis...</p>
          <p className="text-xs mt-1">
            Set AI_PROVIDER and API key in backend .env
          </p>
        </div>
      </div>
    );
  }

  const sentimentColor =
    analysis.marketSentiment.toLowerCase() === "bullish"
      ? "text-profit"
      : analysis.marketSentiment.toLowerCase() === "bearish"
        ? "text-loss"
        : "text-warning";

  const timeStr = new Date(analysis.timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

  return (
    <div className="glass-card-static p-6">
      <div className="flex items-baseline justify-between mb-4">
        <h2 className="text-lg font-semibold text-text-primary">
          AI Analysis
        </h2>
        <span className="text-xs text-text-muted">{timeStr}</span>
      </div>

      {/* Summary */}
      <p className="text-sm text-text-secondary leading-relaxed mb-4">
        {analysis.summary}
      </p>

      {/* Sentiment + Risk */}
      <div className="grid grid-cols-2 gap-3 mb-4">
        <div className="rounded-2xl bg-bg-surface/50 p-3 border border-border-glass">
          <p className="text-xs text-text-muted mb-1">Sentiment</p>
          <p className={`text-sm font-bold ${sentimentColor}`}>
            {analysis.marketSentiment}
          </p>
        </div>
        <div className="rounded-2xl bg-bg-surface/50 p-3 border border-border-glass">
          <p className="text-xs text-text-muted mb-1">Risk</p>
          <p className="text-sm text-text-secondary">
            {analysis.riskAssessment.length > 60
              ? analysis.riskAssessment.slice(0, 57) + "..."
              : analysis.riskAssessment}
          </p>
        </div>
      </div>

      {/* Top Opportunity */}
      {analysis.topOpportunity && (
        <div className="rounded-2xl bg-bg-surface/50 p-3 border border-accent-violet/20">
          <div className="flex items-center gap-2 mb-2">
            <p className="text-xs text-text-muted">Top Signal</p>
            <span className="rounded-lg bg-accent-violet/10 border border-accent-violet/20 px-2 py-0.5 text-xs font-medium text-accent-violet">
              {analysis.topOpportunity.asset}{" "}
              {analysis.topOpportunity.direction}
            </span>
            <span className="text-xs text-text-muted">
              {analysis.topOpportunity.aiConfidence}
            </span>
          </div>
          <p className="text-sm text-text-secondary leading-relaxed">
            {analysis.topOpportunity.reasoning}
          </p>
        </div>
      )}
    </div>
  );
}
