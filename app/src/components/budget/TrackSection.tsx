"use client";

// TrackSection — one of the four feature tiles on /budget.
//
// The "where did the money go?" view. Lists every purchase request in the
// household, newest first, with its lifecycle status, the buyer, the spend
// ceiling, and (once reimbursed) the amount actually paid back. This is the
// shared ledger the admin and every member look at together to answer
// "what's been bought, what's pending, and what's been paid back?".
//
// Pure read-side: consumes the same `useHousehold` polling hook the rest of the
// dashboard uses. No on-chain writes — the actions that change request status
// (approve / reimburse / etc.) live in their own dedicated tiles.

import { useMemo, useState } from "react";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh } from "@/hooks/useRefresh";
import { lamportsToSol, shortPubkey } from "@/lib/format";
import {
  statusFromAnchor,
  STATUS_OPTIONS,
  STATUS_LABELS,
  type Status,
} from "@/lib/types";
import { Avatar } from "@/components/ui/Avatar";
import { StatusBadge } from "@/components/ui/Badge";
import { SectionCard } from "@/components/budget/SectionCard";
import { cn } from "@/lib/cn";

/** Statuses the user can filter the ledger by. "all" is the default. */
type Filter = "all" | Status;

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All" },
  ...STATUS_OPTIONS.map((s) => ({ value: s as Filter, label: STATUS_LABELS[s] })),
];

export function TrackSection() {
  return (
    <SectionCard
      title="Track"
      icon="🧾"
      description="The shared ledger. Every purchase request in the household, its status, the buyer, the spend ceiling, and how much has been paid back. Visible to every member."
    >
      <TrackList />
    </SectionCard>
  );
}

function TrackList() {
  const { household } = useHouseholdContext();
  const { nonce } = useRefresh();
  const state = useHousehold(nonce);
  const [filter, setFilter] = useState<Filter>("all");

  const sorted = useMemo(() => {
    const all = state.data?.requests ?? [];
    return all
      .slice()
      .sort(
        (a, b) =>
          Number(b.account.requestId.toString(10)) -
          Number(a.account.requestId.toString(10)),
      );
  }, [state.data?.requests]);

  const filtered = useMemo(() => {
    if (filter === "all") return sorted;
    return sorted.filter(
      (r) => statusFromAnchor(r.account.status) === filter,
    );
  }, [sorted, filter]);

  if (!household) {
    return (
      <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
        Enter the household admin's address on the dashboard to load the
        ledger.
      </p>
    );
  }
  if (state.loading) {
    return <LedgerSkeleton />;
  }
  if (state.error) {
    return (
      <p className="text-xs text-rose-600 dark:text-rose-400">
        Couldn't load requests: {state.error}
      </p>
    );
  }
  if (sorted.length === 0) {
    return (
      <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
        No purchase requests yet. Once a member creates one from the main
        dashboard, it'll show up here.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {/* Filter pills */}
      <div className="flex flex-wrap gap-1">
        {FILTERS.map((f) => {
          const isActive = filter === f.value;
          return (
            <button
              key={f.value}
              type="button"
              onClick={() => setFilter(f.value)}
              className={cn(
                "rounded-full px-2.5 py-1 text-[11px] font-medium transition-colors",
                isActive
                  ? "bg-emerald-500 text-slate-950"
                  : "bg-stone-100 dark:bg-slate-800/60 text-stone-600 dark:text-slate-300 hover:bg-stone-200 dark:hover:bg-slate-800",
              )}
            >
              {f.label}
            </button>
          );
        })}
      </div>

      {/* List */}
      <div className="flex flex-col gap-2">
        {filtered.length === 0 ? (
          <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
            No requests with status "{STATUS_LABELS[filter as Status]}".
          </p>
        ) : (
          filtered.map((r) => {
            const id = r.account.requestId.toString(10);
            const status = statusFromAnchor(r.account.status) as Status | null;
            const buyer = r.account.buyer;
            const ceilingSol = lamportsToSol(r.account.amountLamports);
            const reimbursedSol = lamportsToSol(r.account.reimbursedAmount);
            const rewardEarned = r.account.rewardEarned.toString(10);
            return (
              <div
                key={id}
                className={cn(
                  "rounded-lg border border-stone-200 dark:border-slate-800",
                  "bg-white dark:bg-slate-950/40 px-4 py-3",
                )}
              >
                <div className="flex flex-wrap items-center gap-3">
                  <Avatar seed={buyer.toBase58()} size="sm" />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                        #{id}
                      </span>
                      {status && <StatusBadge status={status} />}
                      <span className="font-mono text-[10px] text-stone-500 dark:text-slate-500">
                        {shortPubkey(buyer.toBase58(), 4, 4)}
                      </span>
                    </div>
                  </div>
                  <div className="text-right">
                    <div className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-slate-600">
                      Ceiling
                    </div>
                    <div className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                      {ceilingSol} SOL
                    </div>
                  </div>
                  {reimbursedSol !== "0" && (
                    <div className="text-right">
                      <div className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-slate-600">
                        Paid back
                      </div>
                      <div className="text-sm font-semibold text-violet-600 dark:text-violet-400">
                        {reimbursedSol} SOL
                      </div>
                    </div>
                  )}
                  {rewardEarned !== "0" && (
                    <div className="text-right">
                      <div className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-slate-600">
                        Reward
                      </div>
                      <div className="text-sm font-semibold text-amber-600 dark:text-amber-400">
                        +{rewardEarned}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

function LedgerSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: 4 }).map((_, i) => (
        <div
          key={i}
          className="rounded-lg border border-stone-200 dark:border-slate-800 px-4 py-3"
        >
          <div className="flex items-center gap-3">
            <div className="h-6 w-6 animate-pulse rounded-lg bg-stone-200 dark:bg-slate-800" />
            <div className="h-3 w-32 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
            <div className="ml-auto h-3 w-12 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
          </div>
        </div>
      ))}
    </div>
  );
}
