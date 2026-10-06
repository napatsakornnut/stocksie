"use client";

// ReimburseSection — one of the four feature tiles on /budget.
//
// The settlement step of the purchase lifecycle. Maps to a single on-chain
// instruction:
//
//   `reimburse_buyer(lamports)`
//     - Caller  : household Owner or Parent (the approver who green-lit the
//       spend).
//     - Buyer   : the member who actually made the purchase and confirmed
//       restock. The on-chain `buyer` account must match `request.buyer`.
//     - Request : must be in the `restocked` state (only restocked requests can
//       be reimbursed — the program rejects any earlier state).
//     - Amount  : `lamports` ≤ `request.amount_lamports` (the spend ceiling
//       recorded at creation). Hard circuit breaker: MAX_REIMBURSEMENT_LAMPORTS
//       (0.5 SOL).
//
// The buyer earns REWARD_FULL_RUN_COMPLETED (15 points) on success; the vault
// is debited by exactly the reimbursed amount.

import { useMemo, useState } from "react";
import BN from "bn.js";
import { useProgram } from "@/lib/program";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh } from "@/hooks/useRefresh";
import { useTransactionWithRefresh as useTransaction } from "@/hooks/useRefresh";
import { memberPda, purchasePda } from "@/lib/accounts";
import { lamportsToSol, shortPubkey, solToLamports } from "@/lib/format";
import { MAX_REIMBURSEMENT_LAMPORTS } from "@/lib/constants";
import { statusFromAnchor, type Status } from "@/lib/types";
import { Avatar } from "@/components/ui/Avatar";
import { Button } from "@/components/ui/Button";
import { ResultBanner } from "@/components/ui/ResultBanner";
import { SectionCard } from "@/components/budget/SectionCard";
import { cn } from "@/lib/cn";

/** Display ceiling for the amount field (matches the on-chain circuit breaker). */
const MAX_REIMBURSE_SOL = "0.5";

export function ReimburseSection() {
  const { isOwnerConnected } = useHouseholdContext();

  return (
    <SectionCard
      title="Reimburse"
      icon="💸"
      description="Pay a buyer back for something they bought on behalf of the household. Admin or approver only. The request must be marked restocked; the amount can't exceed the request's spending limit (or the 0.5 SOL safety limit)."
    >
      {!isOwnerConnected ? (
        <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
          You're not signed in as this household's admin. Only the admin (or an
          approver) can reimburse buyers from this view.
        </p>
      ) : (
        <ReimburseForm />
      )}
    </SectionCard>
  );
}

function ReimburseForm() {
  const program = useProgram();
  const { household, connectedWallet } = useHouseholdContext();
  const { nonce } = useRefresh();
  const state = useHousehold(nonce);
  const tx = useTransaction();

  // Restocked = ready to be reimbursed. Pending-approval requests are not yet
  // reimbursable; the buyer must have confirmed they restocked first.
  const ready = useMemo(() => {
    const all = state.data?.requests ?? [];
    return all
      .filter((r) => statusFromAnchor(r.account.status) === "restocked")
      .sort(
        (a, b) =>
          Number(b.account.requestId.toString(10)) -
          Number(a.account.requestId.toString(10))
      );
  }, [state.data?.requests]);

  const [selectedId, setSelectedId] = useState<string>("");
  const [amountInput, setAmountInput] = useState<string>("");

  const selected = useMemo(
    () =>
      ready.find((r) => r.account.requestId.toString(10) === selectedId) ??
      null,
    [ready, selectedId]
  );

  const lamports = useMemo(() => solToLamports(amountInput), [amountInput]);

  const amountError = useMemo(() => {
    if (amountInput.trim().length === 0) return null;
    if (lamports === null) return "Enter a valid SOL amount (e.g. 0.05)";
    if (lamports <= 0n) return "Amount must be greater than zero";
    if (lamports > BigInt(MAX_REIMBURSEMENT_LAMPORTS)) {
      return "Exceeds the 0.5 SOL safety limit";
    }
    if (selected) {
      const ceiling = BigInt(selected.account.amountLamports.toString(10));
      if (lamports > ceiling) {
        return "Exceeds this request's spend ceiling";
      }
    }
    return null;
  }, [amountInput, lamports, selected]);

  const canSubmit =
    !!program &&
    !!household &&
    !!connectedWallet &&
    !!selected &&
    lamports !== null &&
    lamports > 0n &&
    lamports <= BigInt(MAX_REIMBURSEMENT_LAMPORTS) &&
    !amountError &&
    !tx.pending;

  const handleSubmit = () => {
    if (!program || !household || !connectedWallet || !selected) return;
    if (lamports === null) return;
    const requestId = BigInt(selected.account.requestId.toString(10));
    const buyer = selected.account.buyer;
    void tx.submit(async () => {
      return program.methods
        .reimburseBuyer(new BN(lamports.toString()))
        .accountsStrict({
          household,
          callerMember: memberPda(household, connectedWallet),
          request: purchasePda(household, requestId),
          buyerMember: memberPda(household, buyer),
          buyer,
          caller: connectedWallet,
        })
        .rpc();
    });
  };

  if (state.loading) {
    return <ReadySkeleton />;
  }
  if (state.error) {
    return (
      <p className="text-xs text-rose-600 dark:text-rose-400">
        Couldn't load requests: {state.error}
      </p>
    );
  }
  if (ready.length === 0) {
    return (
      <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
        No restocked requests waiting for reimbursement. Once a buyer confirms
        they've restocked (from the main dashboard), the request becomes payable
        here.
      </p>
    );
  }

  const ceilingSol = selected
    ? lamportsToSol(selected.account.amountLamports)
    : null;

  return (
    <div className="flex flex-col gap-3">
      {/* Restocked request picker */}
      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
          Pick a restocked request
        </span>
        <div className="flex max-h-44 flex-col gap-1 overflow-y-auto rounded-lg border border-stone-200 dark:border-slate-800 bg-stone-50/40 dark:bg-slate-950/30 p-1">
          {ready.map((r) => {
            const id = r.account.requestId.toString(10);
            const buyer = r.account.buyer;
            const base58 = buyer.toBase58();
            const ceiling = lamportsToSol(r.account.amountLamports);
            const isSelected = selectedId === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => {
                  setSelectedId(id);
                  setAmountInput(ceiling);
                }}
                className={cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-left transition-colors",
                  isSelected
                    ? "bg-emerald-50 dark:bg-emerald-500/10 ring-1 ring-emerald-500/30"
                    : "hover:bg-stone-100 dark:hover:bg-slate-800/60"
                )}
              >
                <Avatar seed={base58} size="md" />
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                    #{id}
                  </div>
                  <div className="truncate font-mono text-[10px] text-stone-500 dark:text-slate-500">
                    {shortPubkey(base58, 4, 4)}
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-slate-600">
                    Ceiling
                  </div>
                  <div className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                    {ceiling} SOL
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      </div>

      {/* Amount field */}
      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
          Amount{ceilingSol ? ` (max ${ceilingSol} SOL)` : ""}
        </span>
        <div className="relative flex items-center">
          <input
            type="number"
            min="0"
            max={MAX_REIMBURSE_SOL}
            step="0.0001"
            inputMode="decimal"
            value={amountInput}
            onChange={(e) => setAmountInput(e.target.value)}
            placeholder={
              selected ? ceilingSol ?? MAX_REIMBURSE_SOL : MAX_REIMBURSE_SOL
            }
            className={cn(
              "w-full rounded-lg border bg-white dark:bg-slate-950/60 px-3 py-2 pr-12 font-mono text-sm text-stone-800 dark:text-slate-100 outline-none transition",
              "placeholder:text-stone-400 dark:placeholder:text-slate-600",
              "focus:border-emerald-500/70 focus:ring-1 focus:ring-emerald-500/40",
              amountError
                ? "border-rose-500/70"
                : "border-stone-200 dark:border-slate-800"
            )}
          />
          <span className="pointer-events-none absolute right-3 text-xs font-medium text-stone-500 dark:text-slate-500">
            SOL
          </span>
        </div>
        {amountError && (
          <p className="text-xs text-rose-600 dark:text-rose-400">
            {amountError}
          </p>
        )}
        <p className="text-[11px] text-stone-500 dark:text-slate-500">
          What they actually spent — may be less than the request's ceiling,
          never more. The buyer earns 15 reward points on success.
        </p>
      </div>

      <Button onClick={handleSubmit} loading={tx.pending} disabled={!canSubmit}>
        Pay back buyer
      </Button>

      <ResultBanner
        pending={tx.pending}
        signature={tx.signature}
        error={tx.error}
        onDismiss={tx.reset}
      />
    </div>
  );
}

function ReadySkeleton() {
  return (
    <div className="flex flex-col gap-2">
      <div className="h-3 w-44 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
      {Array.from({ length: 2 }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 rounded-md px-3 py-2">
          <div className="h-8 w-8 animate-pulse rounded-lg bg-stone-200 dark:bg-slate-800" />
          <div className="h-3 flex-1 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
        </div>
      ))}
    </div>
  );
}
