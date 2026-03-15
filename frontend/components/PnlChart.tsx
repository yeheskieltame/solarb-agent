"use client";

import type { PnlPoint } from "@/lib/types";

interface Props {
  data: PnlPoint[];
}

export default function PnlChart({ data }: Props) {
  if (data.length < 2) {
    return (
      <div className="glass-card-static p-6">
        <h2 className="text-lg font-semibold text-text-primary mb-4">
          P&L History
        </h2>
        <div className="flex items-center justify-center h-48 text-text-muted text-sm">
          Waiting for trade data...
        </div>
      </div>
    );
  }

  const values = data.map((d) => d.cumulative);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const latest = values[values.length - 1];
  const isPositive = latest >= 0;

  const chartWidth = 600;
  const chartHeight = 160;
  const padding = 2;

  const points = data
    .map((d, i) => {
      const x = padding + (i / (data.length - 1)) * (chartWidth - padding * 2);
      const y =
        chartHeight -
        padding -
        ((d.cumulative - min) / range) * (chartHeight - padding * 2);
      return `${x},${y}`;
    })
    .join(" ");

  const areaPoints = `${padding},${chartHeight} ${points} ${chartWidth - padding},${chartHeight}`;
  const strokeColor = isPositive ? "var(--profit)" : "var(--loss)";
  const fillColor = isPositive
    ? "url(#gradientProfit)"
    : "url(#gradientLoss)";

  return (
    <div className="glass-card-static p-6">
      <div className="flex items-baseline justify-between mb-4">
        <h2 className="text-lg font-semibold text-text-primary">
          P&L History
        </h2>
        <span
          className={`text-xl font-bold font-tabular ${isPositive ? "text-profit" : "text-loss"}`}
        >
          {isPositive ? "+" : ""}${latest.toFixed(2)}
        </span>
      </div>

      <svg
        viewBox={`0 0 ${chartWidth} ${chartHeight}`}
        className="w-full h-40"
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id="gradientProfit" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--profit)" stopOpacity="0.3" />
            <stop offset="100%" stopColor="var(--profit)" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="gradientLoss" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--loss)" stopOpacity="0.3" />
            <stop offset="100%" stopColor="var(--loss)" stopOpacity="0" />
          </linearGradient>
        </defs>

        {/* Zero line */}
        {min < 0 && max > 0 && (
          <line
            x1={padding}
            y1={chartHeight - padding - ((-min) / range) * (chartHeight - padding * 2)}
            x2={chartWidth - padding}
            y2={chartHeight - padding - ((-min) / range) * (chartHeight - padding * 2)}
            stroke="var(--text-muted)"
            strokeWidth="0.5"
            strokeDasharray="4,4"
            opacity="0.4"
          />
        )}

        {/* Area fill */}
        <polygon points={areaPoints} fill={fillColor} />

        {/* Line */}
        <polyline
          points={points}
          fill="none"
          stroke={strokeColor}
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </div>
  );
}
