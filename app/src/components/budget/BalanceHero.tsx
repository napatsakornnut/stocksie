"use client";

// BalanceHero — the headline strip at the top of the /budget dashboard.
//
// Surfaces the single number everyone in the household should be able to see at
// a glance — the shared vault balance in SOL — alongside four supporting stats
// (members, pending requests, total reimbursed, total rewards given out) so the
// admin and every member share the exact same picture of "how much money do we
// have, and where has it gone?".
//
// Pure read-side: it consumes the `useHousehold` polling hook (which already
// refreshes instantly on every confirmed write through the shared refresh
// nonce), so this card updates live as approve / reward / track / reimburse
// actions land on-chain. No writes originate from here.

import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh } from "@/hooks/useRefresh";
import {
  lamportsToSol,
  shortPubkey,
} from "@/lib/format";
import {
  statusFromAnchor,
  type Status,
} from "@/lib/types";
import { cn } from "@/lib/cn";

/**
 * Headline balance + four supporting stats for the budget dashboard.
 *
 * Layout: one large "Shared budget" card (the SOL balance) flanked by a row of
 * four compact stat cards. On narrow viewports everything stacks vertically
 * and remains legible.
 */
export function BalanceHero() {
  const { household } = useHouseholdContext();
  const { nonce } = useRefresh();
  const state = useHousehold(nonce);

  if (!household) {
    return (
      <section
        className={cn(
          "overflow-hidden rounded-xl border border-stone-200 dark:border-slate-800",
          "bg-white dark:bg-slate-900/40 shadow-lg shadow-stone-900/5 dark:shadow-slate-950/20",
        )}
      >
        <div className="px-6 py-10 text-center">
          <p className="text-sm text-stone-500 dark:text-slate-400">
            Enter the household admin's address on the dashboard to load your
            shared budget.
          </p>
        </div>
      </section>
    );
  }

  if (state.loading) {
    return <HeroSkeleton />;
  }

  if (state.error) {
    return (
      <section
        className={cn(
          "overflow-hidden rounded-xl border border-rose-300 dark:border-rose-800",
          "bg-rose-50 dark:bg-rose-950/30",
        )}
      >
        <div className="px-6 py-6">
          <p className="text-sm font-medium text-rose-700 dark:text-rose-200">
            Couldn't load the household state.
          </p>
          <p className="mt-1 text-xs text-rose-700/80 dark:text-rose-200/80">
            {state.error}
          </p>
          <button
            type="button"
            onClick={state.refetch}
            className="mt-3 inline-flex h-8 items-center rounded-md border border-rose-300 dark:border-rose-700 px-3 text-xs font-medium text-rose-700 dark:text-rose-200 hover:bg-rose-100 dark:hover:bg-rose-900/40"
          >
            Try again
          </button>
        </div>
      </section>
    );
  }

  const h = state.data?.household;
  if (!h) {
    // Not-yet-initialized household — render a zero-state hero rather than NaNs.
    return (
      <HeroCard
        vaultSol="0"
        refreshing={state.refreshing}
        members={0}
        pending={0}
        reimbursedSol="0"
        totalRewards="0"
        address={household}
        initialized={false}
      />
    );
  }

  // Aggregate across the fetched requests: count pending, sum reimbursed lamports.
  const requests = state.data?.requests ?? [];
  let pendingCount = 0;
  let reimbursedLamports = 0n;
  for (const r of requests) {
    const status = statusFromAnchor(r.account.status) as Status | null;
    if (status === "pending") pendingCount += 1;
    if (r.account.reimbursedAmount) {
      reimbursedLamports += BigInt(r.account.reimbursedAmount.toString(10));
    }
  }

  return (
    <HeroCard
      vaultSol={lamportsToSol(h.vaultBalance)}
      refreshing={state.refreshing}
      members={h.memberCount}
      pending={pendingCount}
      reimbursedSol={lamportsToSol(reimbursedLamports)}
      totalRewards={h.totalRewardsDistributed.toString(10)}
      address={household}
      initialized
    />
  );
}

// ---------------------------------------------------------------------------
// Presentational hero card
// ---------------------------------------------------------------------------

type HeroCardProps = {
  vaultSol: string;
  refreshing: boolean;
  members: number;
  pending: number;
  reimbursedSol: string;
  totalRewards: string;
  address: { toBase58(): string };
  initialized: boolean;
};

function HeroCard({
  vaultSol,
  refreshing,
  members,
  pending,
  reimbursedSol,
  totalRewards,
  address,
  initialized,
}: HeroCardProps) {
  return (
    <section
      className={cn(
        "overflow-hidden rounded-xl border border-stone-200 dark:border-slate-800",
        "bg-gradient-to-br from-white to-stone-50 dark:from-slate-900/60 dark:to-slate-950/40",
        "shadow-lg shadow-stone-900/5 dark:shadow-slate-950/20",
      )}
    >
      <div className="grid gap-px bg-stone-200 dark:bg-slate-800 lg:grid-cols-[1.5fr_2fr]">
        {/* Headline balance */}
        <div className="bg-white dark:bg-slate-900/40 px-6 py-7 sm:px-8">
          <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
            <span aria-hidden="true">💰</span>
            Shared budget
            {refreshing && (
              <span className="ml-1 inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500 dark:bg-emerald-400" />
                updating
              </span>
            )}
          </div>
          <div className="mt-2 flex items-baseline gap-2">
            <span className="text-4xl font-semibold tracking-tight text-stone-800 dark:text-slate-100 sm:text-5xl">
              {vaultSol}
            </span>
            <span className="text-lg font-medium text-stone-500 dark:text-slate-400">
              SOL
            </span>
          </div>
          <p className="mt-2 text-xs text-stone-500 dark:text-slate-500">
            {initialized
              ? "The household vault. Everyone in the household shares this balance."
              : "Household not set up yet — initialize it on the main dashboard."}
          </p>
          <p
            className="mt-3 font-mono text-[10px] text-stone-400 dark:text-slate-600"
            title={address.toBase58()}
          >
            vault · {shortPubkey(address.toBase58(), 6, 6)}
          </p>
        </div>

        {/* Stat grid */}
        <div className="grid grid-cols-2 bg-stone-200 dark:bg-slate-800 sm:grid-cols-4 lg:grid-cols-2 xl:grid-cols-4">
          <StatCell label="Members" value={String(members)} hint="in household" />
          <StatCell
            label="Awaiting approval"
            value={String(pending)}
            hint="open requests"
            accent={pending > 0}
          />
          <StatCell
            label="Reimbursed"
            value={reimbursedSol}
            suffix="SOL"
            hint="paid back, all time"
          />
          <StatCell
            label="Rewards given"
            value={totalRewards}
            hint="points across everyone"
          />
        </div>
      </div>
    </section>
  );
}

function StatCell({
  label,
  value,
  suffix,
  hint,
  accent,
}: {
  label: string;
  value: string;
  suffix?: string;
  hint?: string;
  accent?: boolean;
}) {
  return (
    <div className="bg-white dark:bg-slate-900/40 px-5 py-4">
      <div className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-slate-500">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 truncate text-xl font-semibold tracking-tight",
          accent
            ? "text-amber-600 dark:text-amber-400"
            : "text-stone-800 dark:text-slate-100",
        )}
      >
        {value}
        {suffix && (
          <span className="ml-1 text-xs font-medium text-stone-400 dark:text-slate-500">
            {suffix}
          </span>
        )}
      </div>
      {hint && (
        <div className="mt-0.5 text-[11px] text-stone-500 dark:text-slate-500">
          {hint}
        </div>
      )}
    </div>
  );
}

function HeroSkeleton() {
  return (
    <section className="overflow-hidden rounded-xl border border-stone-200 dark:border-slate-800 bg-white dark:bg-slate-900/40 shadow-lg shadow-stone-900/5 dark:shadow-slate-950/20">
      <div className="grid gap-px bg-stone-200 dark:bg-slate-800 lg:grid-cols-[1.5fr_2fr]">
        <div className="bg-white dark:bg-slate-900/40 px-6 py-7 sm:px-8">
          <div className="h-3 w-24 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
          <div className="mt-4 h-10 w-40 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
          <div className="mt-4 h-3 w-56 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
        </div>
        <div className="grid grid-cols-2 gap-px bg-stone-200 dark:bg-slate-800 sm:grid-cols-4 lg:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div
              key={i}
              className="bg-white dark:bg-slate-900/40 px-5 py-4"
            >
              <div className="h-2.5 w-16 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
              <div className="mt-2 h-6 w-20 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
