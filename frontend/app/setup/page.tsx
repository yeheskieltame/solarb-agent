import Link from "next/link";
import AnimatedBg from "@/components/AnimatedBg";

export default function SetupPage() {
  return (
    <>
      <AnimatedBg variant="dashboard" />

      <div className="relative z-10 min-h-screen">
        {/* Header */}
        <header className="glass-card-static flex items-center justify-between px-6 py-3 mx-4 mt-4 sm:mx-6">
          <Link
            href="/"
            className="text-lg font-bold text-text-primary hover:text-accent-cyan transition-colors"
          >
            SolArb Agent
          </Link>
          <div className="flex gap-3">
            <Link
              href="/dashboard"
              className="glass-card rounded-[var(--radius-button)] px-4 py-2 text-sm font-medium text-text-primary transition-opacity hover:opacity-80"
            >
              Dashboard
            </Link>
          </div>
        </header>

        <main className="max-w-3xl mx-auto px-4 sm:px-6 py-12 space-y-10">
          <div>
            <h1 className="text-4xl font-bold mb-3">
              <span className="text-gradient">Getting Started</span>
            </h1>
            <p className="text-text-secondary">
              Run SolArb Agent locally in under 5 minutes. The agent scans for
              arbitrage opportunities in dry-run mode by default — no wallet or
              funds required.
            </p>
          </div>

          {/* Prerequisites */}
          <Step number="01" title="Prerequisites">
            <p className="text-text-secondary text-sm mb-4">
              Make sure the following tools are installed on your machine:
            </p>
            <div className="grid gap-3 sm:grid-cols-2">
              <PrereqCard
                name="Rust"
                version="1.75+"
                command="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
              />
              <PrereqCard
                name="Node.js"
                version="18+"
                command="https://nodejs.org"
              />
              <PrereqCard
                name="pnpm"
                version="9+"
                command="npm i -g pnpm"
              />
              <PrereqCard
                name="Solana CLI"
                version="2.0+ (optional)"
                command='sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"'
              />
            </div>
          </Step>

          {/* Clone */}
          <Step number="02" title="Clone the Repository">
            <CodeBlock
              lines={[
                "git clone https://github.com/yeheskieltame/solarb-agent.git",
                "cd solarb-agent",
              ]}
            />
          </Step>

          {/* Backend Setup */}
          <Step number="03" title="Setup Backend">
            <p className="text-text-secondary text-sm mb-3">
              Configure environment variables and start the Rust agent:
            </p>
            <CodeBlock
              lines={[
                "cd backend",
                "cp .env.example .env",
                "",
                "# Run the agent (dry-run mode, safe by default)",
                "cargo run",
              ]}
            />
            <p className="text-text-muted text-xs mt-3">
              The agent will start scanning Polymarket and Drift every 3 seconds
              and broadcast data via WebSocket on port 9944.
            </p>
          </Step>

          {/* Frontend Setup */}
          <Step number="04" title="Setup Frontend">
            <p className="text-text-secondary text-sm mb-3">
              In a separate terminal, install dependencies and start the
              dashboard:
            </p>
            <CodeBlock
              lines={[
                "cd frontend",
                "pnpm install",
                "pnpm dev",
              ]}
            />
            <p className="text-text-muted text-xs mt-3">
              Open{" "}
              <span className="text-accent-cyan">http://localhost:3000</span>{" "}
              for the landing page or{" "}
              <span className="text-accent-cyan">
                http://localhost:3000/dashboard
              </span>{" "}
              for the live trading dashboard.
            </p>
          </Step>

          {/* Run Tests */}
          <Step number="05" title="Run Tests">
            <CodeBlock lines={["cd backend", "cargo test"]} />
            <p className="text-text-muted text-xs mt-3">
              Expected: 25 tests passing across scanner, detector, executor,
              risk, and wallet modules.
            </p>
          </Step>

          {/* Optional: Wallet */}
          <Step number="06" title="Create a Solana Wallet (Optional)">
            <p className="text-text-secondary text-sm mb-3">
              Only needed if you want to enable live trading on devnet. Never
              use your main wallet.
            </p>
            <CodeBlock
              lines={[
                "# Generate a dedicated agent keypair",
                "solana-keygen new --outfile ~/.config/solana/solarb-agent.json",
                "",
                "# Fund it on devnet",
                "solana airdrop 2 --keypair ~/.config/solana/solarb-agent.json --url devnet",
                "",
                "# Set the path in backend/.env",
                "# AGENT_KEYPAIR_PATH=~/.config/solana/solarb-agent.json",
                "# DRY_RUN=false",
              ]}
            />
          </Step>

          {/* Architecture overview */}
          <div className="glass-card-static p-6">
            <h3 className="text-lg font-semibold text-text-primary mb-4">
              Architecture Overview
            </h3>
            <div className="grid gap-4 sm:grid-cols-3 text-center">
              <div className="rounded-2xl bg-bg-surface/50 p-4 border border-border-glass">
                <p className="text-sm font-semibold text-accent-cyan">
                  Backend
                </p>
                <p className="text-xs text-text-muted mt-1">
                  Rust agent scans, detects, and executes arbitrage
                </p>
              </div>
              <div className="rounded-2xl bg-bg-surface/50 p-4 border border-accent-cyan/20">
                <p className="text-sm font-semibold text-accent-cyan">
                  WebSocket
                </p>
                <p className="text-xs text-text-muted mt-1">
                  Real-time data stream on port 9944
                </p>
              </div>
              <div className="rounded-2xl bg-bg-surface/50 p-4 border border-border-glass">
                <p className="text-sm font-semibold text-accent-cyan">
                  Frontend
                </p>
                <p className="text-xs text-text-muted mt-1">
                  Next.js dashboard with live updates
                </p>
              </div>
            </div>
          </div>

          {/* CTA */}
          <div className="flex gap-4 pt-4">
            <Link
              href="/dashboard"
              className="rounded-[var(--radius-button)] bg-gradient-to-r from-accent-cyan to-accent-teal px-8 py-3 text-sm font-semibold text-bg-deep transition-opacity hover:opacity-90"
            >
              Open Dashboard
            </Link>
            <a
              href="https://github.com/yeheskieltame/solarb-agent"
              target="_blank"
              rel="noopener noreferrer"
              className="glass-card rounded-[var(--radius-button)] px-8 py-3 text-sm font-semibold text-text-primary transition-opacity hover:opacity-90"
            >
              View Source
            </a>
          </div>
        </main>

        <footer className="relative z-10 px-6 py-12 text-center text-text-muted text-xs border-t border-border-glass">
          <p>
            Built for the Solana Agent Economy Hackathon — Agent Talent Show
          </p>
        </footer>
      </div>
    </>
  );
}

function Step({
  number,
  title,
  children,
}: {
  number: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="glass-card-static p-6">
      <div className="flex items-center gap-3 mb-4">
        <span className="text-xs font-mono text-accent-cyan">{number}</span>
        <h2 className="text-xl font-bold text-text-primary">{title}</h2>
      </div>
      {children}
    </section>
  );
}

function CodeBlock({ lines }: { lines: string[] }) {
  return (
    <div className="rounded-2xl bg-bg-deep/80 border border-border-glass p-4 overflow-x-auto">
      <pre className="text-sm font-mono text-text-secondary leading-relaxed">
        {lines.map((line, i) => (
          <div key={i}>
            {line.startsWith("#") ? (
              <span className="text-text-muted">{line}</span>
            ) : line === "" ? (
              <br />
            ) : (
              <>
                <span className="text-accent-cyan select-none">$ </span>
                {line}
              </>
            )}
          </div>
        ))}
      </pre>
    </div>
  );
}

function PrereqCard({
  name,
  version,
  command,
}: {
  name: string;
  version: string;
  command: string;
}) {
  return (
    <div className="rounded-2xl bg-bg-surface/50 p-4 border border-border-glass">
      <div className="flex items-baseline justify-between mb-2">
        <p className="text-sm font-semibold text-text-primary">{name}</p>
        <p className="text-xs text-text-muted">{version}</p>
      </div>
      <p className="text-xs font-mono text-text-muted break-all">{command}</p>
    </div>
  );
}
