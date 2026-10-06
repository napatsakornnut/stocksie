"use client";

// ApproveSection — one of the four feature tiles on the /budget dashboard.
//
// Lets an admin (or approver) triage the queue of pending purchase requests in
// place — without having to scroll to the full purchase panel on the main
// dashboard. Lists every request currently in the `pending` state with its
// buyer + spend ceiling and an inline Approve / Decline control, so the approver
// can clear the queue from a single view.
//
// Mapping to on-chain instructions:
//   - Approve  → `approve_purchase_request`  (Owner / Parent only)
//   - Decline  → `reject_purchase_request`   (Owner / Parent only, takes a
//                blake3 `reason_hash` — the reason text is private to the UI).
//
// On success the shared refresh nonce bumps, so the hero card, the track list,
// and the main dashboard's StateView all update simultaneously.

import { useMemo, useState } from "react";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh } from "@/hooks/useRefresh";
import { useTransactionWithRefresh as useTransaction } from "@/hooks/useRefresh";
import { useProgram } from "@/lib/program";
import { memberPda, purchasePda } from "@/lib/accounts";
import { toHash32 } from "@/lib/hashes";
import { lamportsToSol, shortPubkey } from "@/lib/format";
import { statusFromAnchor, type Status } from "@/lib/types";
import { Avatar } from "@/components/ui/Avatar";
import { Button } from "@/components/ui/Button";
import { ResultBanner } from "@/components/ui/ResultBanner";
import { SectionCard } from "@/components/budget/SectionCard";
import { cn } from "@/lib/cn";

/**
 * Approve / Decline tile. Renders the pending queue and the two inline actions
 * per row. Falls back to a non-blocking hint when the connected wallet is not
 * the household admin (the program would reject the tx anyway — better to tell
 * the user up front).
 */
export function ApproveSection() {
  const { isOwnerConnected } = useHouseholdContext();

  return (
    <SectionCard
      title="Approve"
      icon="✅"
      description="Triage waiting requests. Admin or approver only — once approved, the buyer can confirm they've restocked and the request moves to reimbursement."
    >
      {!isOwnerConnected ? (
        <RoleGate
          message="You're not signed in as this household's admin. Only the admin (or an approver) can approve or decline requests from this view."
        />
      ) : (
        <ApproveQueue />
      )}
    </SectionCard>
  );
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

function ApproveQueue() {
  const program = useProgram();
  const { household, connectedWallet } = useHouseholdContext();
  const { nonce } = useRefresh();
  const state = useHousehold(nonce);

  const approveTx = useTransaction();
  const rejectTx = useTransaction();
  const [reasonDrafts, setReasonDrafts] = useState<Record<string, string>>({});

  // One shared "busy on this row" signal — disables the sibling button while a
  // tx is in flight so the user can't fire two competing transactions.
  const pendingRowId = approveTx.pending || rejectTx.pending
    ? approveTx.signature ?? rejectTx.signature ?? null
    : null;

  const pending = useMemo(() => {
    const all = state.data?.requests ?? [];
    return all
      .filter((r) => statusFromAnchor(r.account.status) === "pending")
      .sort(
        (a, b) =>
          Number(b.account.requestId.toString(10)) -
          Number(a.account.requestId.toString(10)),
      );
  }, [state.data?.requests]);

  if (state.loading) {
    return <QueueSkeleton />;
  }
  if (state.error) {
    return (
      <p className="text-xs text-rose-600 dark:text-rose-400">
        Couldn't load requests: {state.error}
      </p>
    );
  }
  if (pending.length === 0) {
    return (
      <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
        No requests waiting. New purchase requests from any household member
        will appear here for approval.
      </p>
    );
  }

  const submitApprove = (requestId: bigint) => {
    if (!program || !household || !connectedWallet) return;
    void approveTx.submit(async () => {
      return program.methods
        .approvePurchaseRequest()
        .accountsStrict({
          household,
          callerMember: memberPda(household, connectedWallet),
          request: purchasePda(household, requestId),
          caller: connectedWallet,
        })
        .rpc();
    });
  };

  const submitReject = (requestId: bigint) => {
    if (!program || !household || !connectedWallet) return;
    const reason = (reasonDrafts[requestId.toString()] ?? "").trim();
    const reasonHash = toHash32(reason.length > 0 ? reason : "Declined");
    void rejectTx.submit(async () => {
      return program.methods
        .rejectPurchaseRequest(reasonHash)
        .accountsStrict({
          household,
          callerMember: memberPda(household, connectedWallet),
          request: purchasePda(household, requestId),
          caller: connectedWallet,
        })
        .rpc();
    });
  };

  return (
    <div className="flex flex-col gap-3">
      {pending.map((r) => {
        const id = r.account.requestId.toString(10);
        const buyer = r.account.buyer;
        const ceilingSol = lamportsToSol(r.account.amountLamports);
        const isLocked =
          (approveTx.pending || rejectTx.pending) && pendingRowId === null;
        return (
          <div
            key={id}
            className={cn(
              "rounded-lg border border-stone-200 dark:border-slate-800 bg-white dark:bg-slate-950/40",
              "px-4 py-3",
            )}
          >
            <div className="flex flex-wrap items-center gap-3">
              <Avatar seed={buyer.toBase58()} size="md" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                    Request #{id}
                  </span>
                  <span className="text-xs text-stone-500 dark:text-slate-400">
                    · {shortPubkey(buyer.toBase58(), 4, 4)}
                  </span>
                </div>
                <div className="mt-0.5 text-xs text-stone-500 dark:text-slate-400">
                  Spend ceiling{" "}
                  <span className="font-medium text-stone-700 dark:text-slate-200">
                    {ceilingSol} SOL
                  </span>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => submitApprove(BigInt(id))}
                  loading={approveTx.pending}
                  disabled={rejectTx.pending || isLocked}
                >
                  Approve
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() => submitReject(BigInt(id))}
                  loading={rejectTx.pending}
                  disabled={approveTx.pending || isLocked}
                >
                  Decline
                </Button>
              </div>
            </div>

            {/* Optional reason for declining — kept private, hashed on submit. */}
            <input
              type="text"
              value={reasonDrafts[id] ?? ""}
              onChange={(e) =>
                setReasonDrafts((prev) => ({ ...prev, [id]: e.target.value }))
              }
              placeholder='Optional reason for declining (private — only a hash goes on-chain)'
              className={cn(
                "mt-3 w-full rounded-md border border-stone-200 dark:border-slate-800",
                "bg-white dark:bg-slate-950/60 px-3 py-1.5 text-xs text-stone-800 dark:text-slate-100",
                "placeholder:text-stone-400 dark:placeholder:text-slate-600 outline-none",
                "focus:border-emerald-500/70 focus:ring-1 focus:ring-emerald-500/40",
              )}
            />
          </div>
        );
      })}

      <ResultBanner
        pending={approveTx.pending || rejectTx.pending}
        signature={approveTx.signature ?? rejectTx.signature}
        error={approveTx.error ?? rejectTx.error}
        onDismiss={() => {
          approveTx.reset();
          rejectTx.reset();
        }}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function RoleGate({ message }: { message: string }) {
  return (
    <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
      {message}
    </p>
  );
}

function QueueSkeleton() {
  return (
    <div className="flex flex-col gap-3">
      {Array.from({ length: 2 }).map((_, i) => (
        <div
          key={i}
          className="rounded-lg border border-stone-200 dark:border-slate-800 px-4 py-3"
        >
          <div className="flex items-center gap-3">
            <div className="h-8 w-8 animate-pulse rounded-lg bg-stone-200 dark:bg-slate-800" />
            <div className="h-3 w-32 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
          </div>
          <div className="mt-3 h-7 w-full animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
        </div>
      ))}
    </div>
  );
}
