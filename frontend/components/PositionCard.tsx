"use client";

import type { Position } from "@/lib/types";

interface Props {
  positions: Position[];
}

export default function PositionCard({ positions }: Props) {
  return (
    <div className="glass-card-static p-6">
      <h2 className="text-lg font-semibold text-text-primary mb-4">
        Open Positions
      </h2>

      {positions.length === 0 ? (
        <p className="text-sm text-text-muted py-8 text-center">
          No open positions
        </p>
      ) : (
        <div className="space-y-3">
          {positions.map((pos) => (
            <PositionRow key={pos.id} position={pos} />
          ))}
        </div>
      )}
    </div>
  );
}

function PositionRow({ position }: { position: Position }) {
  const isProfitable = position.pnl >= 0;
  const pnlColor = isProfitable ? "text-profit" : "text-loss";
  const sideColor =
    position.side === "Long" ? "text-profit" : "text-loss";

  const openedStr = new Date(position.openedAt).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });

  return (
    <div className="flex items-center gap-4 rounded-2xl bg-bg-surface/50 px-4 py-3 border border-border-glass">
      {/* Asset + Side */}
      <div className="min-w-[70px]">
        <p className="text-sm font-semibold text-text-primary">
          {position.asset}
        </p>
        <p className={`text-xs font-medium ${sideColor}`}>
          {position.side}
        </p>
      </div>

      {/* Prices */}
      <div className="flex-1 grid grid-cols-2 gap-2 text-center">
        <div>
          <p className="text-xs text-text-muted">Entry</p>
          <p className="text-sm font-tabular text-text-secondary">
            ${position.entryPrice.toLocaleString()}
          </p>
        </div>
        <div>
          <p className="text-xs text-text-muted">Current</p>
          <p className="text-sm font-tabular text-text-primary">
            ${position.currentPrice.toLocaleString()}
          </p>
        </div>
      </div>

      {/* Size */}
      <div className="text-center min-w-[60px]">
        <p className="text-xs text-text-muted">Size</p>
        <p className="text-sm font-tabular text-text-secondary">
          ${position.sizeUsdc.toFixed(0)}
        </p>
      </div>

      {/* P&L */}
      <div className="text-right min-w-[80px]">
        <p className={`text-sm font-bold font-tabular ${pnlColor}`}>
          {isProfitable ? "+" : ""}${position.pnl.toFixed(2)}
        </p>
        <p className={`text-xs font-tabular ${pnlColor}`}>
          {isProfitable ? "+" : ""}
          {position.pnlPercent.toFixed(1)}%
        </p>
      </div>

      {/* Time */}
      <p className="text-xs text-text-muted min-w-[40px] text-right">
        {openedStr}
      </p>
    </div>
  );
}
