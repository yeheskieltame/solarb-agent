"use client";

import type { AgentStatus } from "@/lib/types";

interface Props {
  status: AgentStatus;
}

export default function AgentStats({ status }: Props) {
  const uptimeStr = formatUptime(status.uptime);
  const lastScanStr = formatAgo(status.lastScan);

  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
      <Card
        label="Status"
        value={status.isRunning ? "Active" : "Stopped"}
        accent={status.isRunning ? "profit" : "loss"}
      />
      <Card label="Scans" value={status.scanCount.toLocaleString()} />
      <Card
        label="Opportunities"
        value={status.opportunitiesFound.toLocaleString()}
      />
      <Card
        label="Trades"
        value={status.tradesExecuted.toLocaleString()}
      />
      <Card
        label="Total P&L"
        value={`$${status.totalPnl.toFixed(2)}`}
        accent={status.totalPnl >= 0 ? "profit" : "loss"}
      />
      <Card label="Uptime" value={uptimeStr} />
      <Card label="Last Scan" value={lastScanStr} />
      <Card
        label="Mode"
        value={status.mode}
        accent={status.mode === "Live" ? "profit" : "warning"}
      />
    </div>
  );
}

function Card({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: "profit" | "loss" | "warning";
}) {
  const accentClass =
    accent === "profit"
      ? "text-profit"
      : accent === "loss"
        ? "text-loss"
        : accent === "warning"
          ? "text-warning"
          : "text-text-primary";

  return (
    <div className="glass-card p-4">
      <p className="text-xs text-text-muted uppercase tracking-wider">
        {label}
      </p>
      <p className={`mt-1 text-xl font-bold font-tabular ${accentClass}`} suppressHydrationWarning>
        {value}
      </p>
    </div>
  );
}

function formatUptime(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function formatAgo(timestamp: number): string {
  const diff = Math.floor((Date.now() - timestamp) / 1000);
  if (diff < 5) return "Just now";
  if (diff < 60) return `${diff}s ago`;
  return `${Math.floor(diff / 60)}m ago`;
}
