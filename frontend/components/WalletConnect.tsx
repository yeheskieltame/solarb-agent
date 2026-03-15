"use client";

import { useState, useCallback } from "react";

export default function WalletConnect() {
  const [address, setAddress] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);

  const connect = useCallback(async () => {
    setConnecting(true);
    try {
      const provider = getBitgetProvider();
      if (!provider) {
        window.open("https://web3.bitget.com", "_blank");
        return;
      }
      const resp = await provider.connect();
      const pubkey = resp.publicKey?.toString();
      if (pubkey) setAddress(pubkey);
    } catch {
      /* user rejected */
    } finally {
      setConnecting(false);
    }
  }, []);

  const disconnect = useCallback(() => {
    const provider = getBitgetProvider();
    provider?.disconnect?.();
    setAddress(null);
  }, []);

  if (address) {
    return (
      <button
        onClick={disconnect}
        className="glass-card rounded-[var(--radius-button)] px-4 py-2 text-sm font-medium text-text-primary transition-opacity hover:opacity-80"
      >
        {truncate(address)}
      </button>
    );
  }

  return (
    <button
      onClick={connect}
      disabled={connecting}
      className="rounded-[var(--radius-button)] bg-gradient-to-r from-accent-cyan to-accent-teal px-5 py-2 text-sm font-semibold text-bg-deep transition-opacity hover:opacity-90 disabled:opacity-50"
    >
      {connecting ? "Connecting..." : "Connect Wallet"}
    </button>
  );
}

function truncate(addr: string): string {
  return `${addr.slice(0, 4)}...${addr.slice(-4)}`;
}

interface BitgetProvider {
  connect: () => Promise<{ publicKey?: { toString: () => string } }>;
  disconnect?: () => void;
}

function getBitgetProvider(): BitgetProvider | null {
  if (typeof window === "undefined") return null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  return w.bitkeep?.solana ?? w.phantom?.solana ?? null;
}
