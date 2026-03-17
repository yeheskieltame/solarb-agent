import Image from "next/image";
import Link from "next/link";
import AnimatedBg from "@/components/AnimatedBg";
import Hero from "@/components/Hero";

export default function Home() {
  return (
    <>
      <AnimatedBg variant="landing" />
      <Hero />

      {/* Section Divider — bioluminescent river */}
      <div className="relative z-10 h-64 sm:h-80 -mt-16 overflow-hidden">
        <Image
          src="/bg/section-divider.webp"
          alt=""
          fill
          className="object-cover object-center"
          sizes="100vw"
        />
        <div className="absolute inset-0 bg-gradient-to-b from-bg-deep via-transparent to-bg-deep" />
      </div>

      {/* How It Works */}
      <section className="relative z-10 px-6 py-24 max-w-5xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-16">
          <span className="text-gradient">How It Works</span>
        </h2>

        <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
          <FlowCard
            step="01"
            title="Scan"
            description="Pulls live odds from Polymarket and funding rates from Drift every 3 seconds."
          />
          <FlowCard
            step="02"
            title="Detect"
            description="Cross-matches signals, calculates net spread after all venue fees."
          />
          <FlowCard
            step="03"
            title="Score"
            description="Assigns confidence level based on spread magnitude and market depth."
          />
          <FlowCard
            step="04"
            title="Execute"
            description="Places both legs atomically: prediction token + perp position."
          />
          <FlowCard
            step="05"
            title="Monitor"
            description="Tracks P&L in real-time, enforces take-profit and stop-loss levels."
          />
          <FlowCard
            step="06"
            title="Protect"
            description="Risk gates enforce position limits, daily loss stops, and exposure caps."
          />
        </div>
      </section>

      {/* Venues */}
      <section className="relative z-10 px-6 py-24 max-w-4xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-16">
          <span className="text-gradient-violet">Cross-Venue Arbitrage</span>
        </h2>

        <div className="grid gap-6 sm:grid-cols-2">
          <div className="glass-card p-8">
            <p className="text-xs text-text-muted uppercase tracking-wider mb-2">
              Signal Source
            </p>
            <h3 className="text-xl font-bold text-accent-violet mb-3">
              Polymarket
            </h3>
            <p className="text-sm text-text-secondary leading-relaxed">
              Prediction market odds via CLOB API. YES token price equals
              implied probability of an event occurring.
            </p>
          </div>

          <div className="glass-card p-8">
            <p className="text-xs text-text-muted uppercase tracking-wider mb-2">
              Hedge Venue
            </p>
            <h3 className="text-xl font-bold text-accent-cyan mb-3">
              Drift Protocol
            </h3>
            <p className="text-sm text-text-secondary leading-relaxed">
              Perpetual futures on Solana. Funding rates and mark premium
              reveal directional sentiment for the same asset.
            </p>
          </div>
        </div>

        {/* Spread model */}
        <div className="glass-card-static p-8 mt-6">
          <h3 className="text-lg font-bold text-text-primary mb-6">
            Spread Calculation
          </h3>
          <div className="grid grid-cols-3 gap-4 text-center">
            <div className="rounded-2xl bg-bg-surface/50 p-4 border border-border-glass">
              <p className="text-xs text-text-muted mb-1">Low</p>
              <p className="text-lg font-bold text-text-muted font-tabular">
                2.5 - 3.5%
              </p>
              <p className="text-xs text-text-muted mt-1">Log only</p>
            </div>
            <div className="rounded-2xl bg-bg-surface/50 p-4 border border-warning/20">
              <p className="text-xs text-warning mb-1">Medium</p>
              <p className="text-lg font-bold text-warning font-tabular">
                3.5 - 6.0%
              </p>
              <p className="text-xs text-text-muted mt-1">Queue</p>
            </div>
            <div className="rounded-2xl bg-bg-surface/50 p-4 border border-profit/20">
              <p className="text-xs text-profit mb-1">High</p>
              <p className="text-lg font-bold text-profit font-tabular">
                &gt; 6.0%
              </p>
              <p className="text-xs text-text-muted mt-1">Auto-execute</p>
            </div>
          </div>
        </div>
      </section>

      {/* Section Divider 2 — river scene again */}
      <div className="relative z-10 h-48 sm:h-64 overflow-hidden">
        <Image
          src="/bg/section-divider.webp"
          alt=""
          fill
          className="object-cover object-bottom"
          sizes="100vw"
        />
        <div className="absolute inset-0 bg-gradient-to-b from-bg-deep via-transparent to-bg-deep" />
      </div>

      {/* Tech Stack */}
      <section className="relative z-10 px-6 py-24 max-w-4xl mx-auto">
        <h2 className="text-3xl font-bold text-center mb-16">
          <span className="text-gradient">Tech Stack</span>
        </h2>

        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <TechCard title="Rust + Tokio" detail="Agent Runtime" />
          <TechCard title="Solana" detail="Blockchain" />
          <TechCard title="Jupiter" detail="DEX Routing" />
          <TechCard title="Drift" detail="Perpetuals" />
          <TechCard title="Polymarket" detail="Predictions" />
          <TechCard title="Bitget Wallet" detail="Wallet SDK" />
          <TechCard title="Next.js" detail="Dashboard" />
          <TechCard title="WebSocket" detail="Real-time Data" />
        </div>
      </section>

      {/* Ocean bottom */}
      <div className="relative z-10 h-64 sm:h-80 overflow-hidden">
        <Image
          src="/bg/landing-page-bottom.webp"
          alt=""
          fill
          className="object-cover object-top"
          sizes="100vw"
        />
        <div className="absolute inset-0 bg-gradient-to-b from-bg-deep via-transparent to-bg-deep/80" />
      </div>

      {/* Footer */}
      <footer className="relative z-10 px-6 py-12 text-center border-t border-border-glass">
        <div className="flex flex-wrap justify-center gap-6 mb-4">
          <Link
            href="/dashboard"
            className="text-sm text-text-secondary hover:text-accent-cyan transition-colors"
          >
            Dashboard
          </Link>
          <Link
            href="/setup"
            className="text-sm text-text-secondary hover:text-accent-cyan transition-colors"
          >
            Getting Started
          </Link>
          <a
            href="https://github.com/yeheskieltame/solarb-agent"
            target="_blank"
            rel="noopener noreferrer"
            className="text-sm text-text-secondary hover:text-accent-cyan transition-colors"
          >
            GitHub
          </a>
        </div>
        <p className="text-text-muted text-xs">
          Built for the Solana Agent Economy Hackathon — Agent Talent Show
        </p>
      </footer>
    </>
  );
}

function FlowCard({
  step,
  title,
  description,
}: {
  step: string;
  title: string;
  description: string;
}) {
  return (
    <div className="glass-card p-6">
      <span className="text-xs text-accent-cyan font-mono">{step}</span>
      <h3 className="text-lg font-bold text-text-primary mt-2">{title}</h3>
      <p className="text-sm text-text-secondary mt-2 leading-relaxed">
        {description}
      </p>
    </div>
  );
}

function TechCard({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="glass-card p-5 text-center">
      <p className="text-sm font-semibold text-text-primary">{title}</p>
      <p className="text-xs text-text-muted mt-1">{detail}</p>
    </div>
  );
}
