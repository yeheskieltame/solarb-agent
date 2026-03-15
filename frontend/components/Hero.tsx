import Image from "next/image";
import Link from "next/link";

export default function Hero() {
  return (
    <section className="relative z-10 flex min-h-screen flex-col items-center justify-center px-6 text-center">
      {/* Agent character */}
      <div className="relative mb-6 h-44 w-44 sm:h-52 sm:w-52 animate-float">
        <Image
          src="/bg/agent-character.webp"
          alt="SolArb Agent"
          fill
          priority
          className="object-contain drop-shadow-[0_0_40px_rgba(56,189,248,0.3)]"
          sizes="(max-width: 640px) 176px, 208px"
        />
      </div>

      <h1 className="text-5xl font-bold tracking-tight sm:text-6xl lg:text-7xl">
        <span className="text-gradient">SolArb</span>{" "}
        <span className="text-text-primary">Agent</span>
      </h1>

      <p className="mt-5 max-w-xl text-lg text-text-secondary leading-relaxed">
        Autonomous cross-venue arbitrage between Polymarket odds and Drift
        perpetual funding rates on Solana.
      </p>

      {/* Stats row */}
      <div className="mt-10 flex gap-8 sm:gap-12">
        <StatItem label="Scan Cycle" value="3s" />
        <StatItem label="Venues" value="2" />
        <StatItem label="Min Spread" value="2.5%" />
      </div>

      {/* CTA */}
      <div className="mt-12 flex gap-4">
        <Link
          href="/dashboard"
          className="rounded-[var(--radius-button)] bg-gradient-to-r from-accent-cyan to-accent-teal px-8 py-3 text-sm font-semibold text-bg-deep transition-opacity hover:opacity-90"
        >
          Open Dashboard
        </Link>
        <a
          href="https://github.com"
          target="_blank"
          rel="noopener noreferrer"
          className="glass-card rounded-[var(--radius-button)] px-8 py-3 text-sm font-semibold text-text-primary transition-opacity hover:opacity-90"
        >
          View Source
        </a>
      </div>

      {/* Scroll indicator */}
      <div className="absolute bottom-8 flex flex-col items-center gap-2 text-text-muted text-xs">
        <span>Scroll</span>
        <div className="h-8 w-px bg-gradient-to-b from-text-muted to-transparent" />
      </div>
    </section>
  );
}

function StatItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col items-center">
      <span className="text-2xl font-bold text-gradient font-tabular">
        {value}
      </span>
      <span className="mt-1 text-xs text-text-muted uppercase tracking-wider">
        {label}
      </span>
    </div>
  );
}
