"use client";

// BudgetDashboard — the single-page budget + payment hub for the household.
//
// Sits on the `/budget` route and stitches together the four actions a
// household needs to coordinate money across admin + members:
//
//   1. Total SOL balance (the hero card) — the single shared number everyone
//      sees first.
//   2. Approve  — admin / approver triages pending requests.
//   3. Reward   — admin / approver hands out reward points.
//   4. Track    — every member reads the shared purchase ledger.
//   5. Reimburse— admin / approver pays a restocked buyer back.
//
// Every tile consumes the same `useHousehold` polling hook + the shared
// `useRefresh` nonce, so when any action lands on-chain, the hero stats, the
// ledger, and every action tile update together — no per-tile refetch wiring.
//
// The four action tiles share the same `SectionCard` shell so the dashboard
// reads as a clean 2x2 grid. On narrow viewports they stack to a single column.

import { useEffect, useState } from "react";
import Link from "next/link";
import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { WalletButton } from "@/components/WalletButton";
import { ThemeToggle } from "@/components/ui/ThemeToggle";
import { BalanceHero } from "@/components/budget/BalanceHero";
import { ApproveSection } from "@/components/budget/ApproveSection";
import { RewardSection } from "@/components/budget/RewardSection";
import { TrackSection } from "@/components/budget/TrackSection";
import { ReimburseSection } from "@/components/budget/ReimburseSection";

export function BudgetDashboard() {
  const { connection } = useConnection();
  const { connected, publicKey, wallet } = useWallet();
  const { ownerInput, setOwnerInput, ownerInputError, household } =
    useHouseholdContext();

  // SSR-safety: wallet state (connected, publicKey, wallet.adapter) is only
  // populated on the client after the wallet-standard adapter mounts. Rendering
  // from that state during SSR would mismatch the client (the " · " footer and
  // the status pill would appear only on the client), throwing the hydration
  // error. We gate wallet-derived UI on `mounted` so server + pre-mount client
  // render the same "disconnected" shell. Same pattern as WalletButton.
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const endpoint = connection.rpcEndpoint;
  const shortAddr =
    mounted && publicKey && connected
      ? `${publicKey.toBase58().slice(0, 6)}…${publicKey.toBase58().slice(-4)}`
      : null;

  return (
    <main className="mx-auto max-w-6xl px-6 py-10">
      {/* ----------------------------------------------------------------- */}
      {/* Header — back link, title, wallet control                         */}
      {/* ----------------------------------------------------------------- */}
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <Link
            href="/"
            className="inline-flex w-fit items-center gap-1 text-xs font-medium text-stone-500 transition-colors hover:text-stone-700 dark:text-slate-400 dark:hover:text-slate-200"
          >
            <span aria-hidden="true">←</span> Back to dashboard
          </Link>
          <h1 className="flex items-center gap-2 text-2xl font-semibold tracking-tight text-stone-800 dark:text-slate-100">
            <span aria-hidden="true">📊</span>
            Budget
          </h1>
          <p className="text-sm text-stone-500 dark:text-slate-400">
            One shared view for the household money — the total balance, pending
            approvals, rewards, and payments.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <ThemeToggle />
          <WalletButton />
        </div>
      </header>

      {/* ----------------------------------------------------------------- */}
      {/* Status pill + admin address field                                 */}
      {/* ----------------------------------------------------------------- */}
      <section className="mt-4 flex flex-wrap items-center gap-2">
        {mounted && connected && shortAddr ? (
          <div className="inline-flex items-center gap-2 rounded-full border border-emerald-500/30 bg-emerald-50 px-3 py-1 text-xs text-emerald-600 dark:bg-emerald-500/10 dark:text-emerald-300">
            <span
              className="h-1.5 w-1.5 rounded-full bg-emerald-500 dark:bg-emerald-400"
              aria-hidden="true"
            />
            <span className="font-medium">Signed in</span>
            <span className="text-emerald-600/60 dark:text-emerald-400/60">
              ·
            </span>
            <code className="font-mono text-[11px] text-emerald-700/80 dark:text-emerald-200/80">
              {shortAddr}
            </code>
          </div>
        ) : (
          <div className="inline-flex items-center gap-2 rounded-full border border-stone-300 bg-white px-3 py-1 text-xs text-stone-500 dark:border-slate-700 dark:bg-slate-900/40 dark:text-slate-400">
            <span
              className="h-1.5 w-1.5 rounded-full bg-slate-400 dark:bg-slate-500"
              aria-hidden="true"
            />
            <span>Not signed in — sign in to act</span>
          </div>
        )}
      </section>

      {/* Admin address override — same control the main dashboard's StateView
          exposes. Keeping it here so members can view another household's
          budget without bouncing back to /. */}
      <section className="mt-3">
        <AdminAddressField
          value={ownerInput}
          onChange={setOwnerInput}
          error={ownerInputError}
          resolved={household?.toBase58() ?? null}
        />
      </section>

      {/* ----------------------------------------------------------------- */}
      {/* Total SOL balance hero                                            */}
      {/* ----------------------------------------------------------------- */}
      <div className="mt-6">
        <BalanceHero />
      </div>

      {/* ----------------------------------------------------------------- */}
      {/* Four feature tiles (2 x 2 grid on lg, stacked on narrow)          */}
      {/* ----------------------------------------------------------------- */}
      <div className="mt-6 grid gap-6 lg:grid-cols-2">
        <ApproveSection />
        <RewardSection />
        <TrackSection />
        <ReimburseSection />
      </div>

      {/* ----------------------------------------------------------------- */}
      {/* Footer hint                                                       */}
      {/* ----------------------------------------------------------------- */}
      <footer className="mt-10 border-t border-stone-200 pt-5 text-xs leading-relaxed text-stone-500 dark:border-slate-800 dark:text-slate-500">
        <p>
          Every action here is recorded on Solana against the household shown
          above. The admin and every member see the same balances and the same
          ledger — the on-chain state is the single source of truth.
        </p>
        {mounted && connected && shortAddr && (
          <p className="mt-2">
            You are signed in as{" "}
            <code className="font-mono text-emerald-600 dark:text-emerald-400">
              {shortAddr}
            </code>
            .
          </p>
        )}
        <p className="mt-2 text-[11px] text-stone-400 dark:text-slate-600">
          Cluster:{" "}
          <code className="font-mono">
            {endpoint.includes("127.0.0.1") || endpoint.includes("localhost")
              ? "localnet"
              : endpoint}
          </code>
          {mounted && wallet?.adapter && (
            <>
              {" · "}
              <code className="font-mono">{wallet.adapter.name}</code>
            </>
          )}
        </p>
      </footer>
    </main>
  );
}

// ---------------------------------------------------------------------------
// Admin-address field — mirrors the one in StateView so members can point at
// another household without leaving /budget.
// ---------------------------------------------------------------------------

function AdminAddressField({
  value,
  onChange,
  error,
  resolved,
}: {
  value: string;
  onChange: (v: string) => void;
  error: string | null;
  resolved: string | null;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor="budget-admin-address"
        className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400"
      >
        Household admin address
      </label>
      <input
        id="budget-admin-address"
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="The admin's wallet address (defaults to your account)"
        spellCheck={false}
        className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 font-mono text-xs tracking-tight text-stone-800 outline-none transition placeholder:text-stone-400 focus:border-emerald-500/70 focus:ring-1 focus:ring-emerald-500/40 dark:border-slate-800 dark:bg-slate-950/60 dark:text-slate-100 dark:placeholder:text-slate-600"
      />
      <div className="flex flex-wrap items-center gap-2 text-[11px]">
        {error ? (
          <span className="text-rose-600 dark:text-rose-400">{error}</span>
        ) : (
          <span className="text-stone-500 dark:text-slate-500">
            Your household is tied to its admin's address. Change this to view
            another household.
          </span>
        )}
        {resolved && (
          <code
            className="font-mono text-stone-400 dark:text-slate-600"
            title={resolved}
          >
            loaded · {resolved.slice(0, 6)}…{resolved.slice(-4)}
          </code>
        )}
      </div>
    </div>
  );
}
