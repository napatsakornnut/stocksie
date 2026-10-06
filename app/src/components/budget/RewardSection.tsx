"use client";

// RewardSection — one of the four feature tiles on /budget.
//
// Lets the admin (or approver) hand out reward points to any active household
// member, with the member roster pre-fetched so the admin can pick from a list
// instead of pasting a wallet address. Maps to a single on-chain instruction:
//
//   `award_reward(member_wallet, points, reason_hash)`
//     - Owner / Parent only (gated by `isOwnerConnected` in the UI; the program
//       re-checks via `caller_member.can_approve()`).
//     - The reason text never leaves the browser — a blake3 fingerprint is what
//       goes on-chain (`toHash32`).
//
// On success the shared refresh nonce bumps, so the hero card's "Rewards given"
// counter and the main dashboard's member roster update live.

import { useMemo, useState } from "react";
import BN from "bn.js";
import { PublicKey } from "@solana/web3.js";
import { useProgram } from "@/lib/program";
import { useHouseholdContext } from "@/hooks/useHouseholdContext";
import { useHousehold } from "@/hooks/useHousehold";
import { useRefresh } from "@/hooks/useRefresh";
import { useTransactionWithRefresh as useTransaction } from "@/hooks/useRefresh";
import { memberPda } from "@/lib/accounts";
import { toHash32 } from "@/lib/hashes";
import { roleFromAnchor, type Role } from "@/lib/types";
import { Avatar } from "@/components/ui/Avatar";
import { Button } from "@/components/ui/Button";
import { ResultBanner } from "@/components/ui/ResultBanner";
import { SectionCard } from "@/components/budget/SectionCard";
import { cn } from "@/lib/cn";

export function RewardSection() {
  const { isOwnerConnected } = useHouseholdContext();

  return (
    <SectionCard
      title="Reward"
      icon="🎁"
      description="Give reward points to a member for any reason. Admin or approver only. The reason is kept private — only a scrambled fingerprint is recorded on-chain."
    >
      {!isOwnerConnected ? (
        <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
          You're not signed in as this household's admin. Only the admin (or an
          approver) can give reward points.
        </p>
      ) : (
        <AwardForm />
      )}
    </SectionCard>
  );
}

function AwardForm() {
  const program = useProgram();
  const { household, connectedWallet } = useHouseholdContext();
  const { nonce } = useRefresh();
  const state = useHousehold(nonce);
  const tx = useTransaction();

  const [selectedWallet, setSelectedWallet] = useState<string>("");
  const [pointsInput, setPointsInput] = useState<string>("");
  const [reasonInput, setReasonInput] = useState<string>("");

  // Active members only, owner-first, then by joined slot for stable ordering.
  const members = useMemo(() => {
    const all = state.data?.members ?? [];
    return all
      .filter((m) => m.account.active)
      .sort((a, b) => {
        const ra = roleFromAnchor(a.account.role) as Role | null;
        const rb = roleFromAnchor(b.account.role) as Role | null;
        if (ra === "owner" && rb !== "owner") return -1;
        if (rb === "owner" && ra !== "owner") return 1;
        return (
          Number(a.account.joinedSlot.toString(10)) -
          Number(b.account.joinedSlot.toString(10))
        );
      });
  }, [state.data?.members]);

  const points = useMemo(() => {
    const trimmed = pointsInput.trim();
    if (trimmed.length === 0) return null;
    if (!/^\d+$/.test(trimmed)) return null;
    try {
      return BigInt(trimmed);
    } catch {
      return null;
    }
  }, [pointsInput]);

  const pointsError = useMemo(() => {
    if (pointsInput.trim().length === 0) return null;
    if (points === null) return "Enter a whole number of points";
    if (points <= 0n) return "Points must be greater than zero";
    return null;
  }, [pointsInput, points]);

  const canSubmit =
    !!program &&
    !!household &&
    !!connectedWallet &&
    selectedWallet.length > 0 &&
    points !== null &&
    points > 0n &&
    reasonInput.trim().length > 0 &&
    !pointsError &&
    !tx.pending;

  const handleSubmit = () => {
    if (!program || !household || !connectedWallet) return;
    if (selectedWallet.length === 0 || points === null) return;
    if (reasonInput.trim().length === 0) return;

    let target: PublicKey;
    try {
      target = new PublicKey(selectedWallet);
    } catch {
      return;
    }
    const targetMember = memberPda(household, target);
    const reasonHash = toHash32(reasonInput);
    void tx.submit(async () => {
      return program.methods
        .awardReward(target, new BN(points.toString()), reasonHash)
        .accountsStrict({
          household,
          callerMember: memberPda(household, connectedWallet),
          targetMember,
          caller: connectedWallet,
        })
        .rpc();
    });
  };

  if (state.loading) {
    return <MemberSkeleton />;
  }
  if (state.error) {
    return (
      <p className="text-xs text-rose-600 dark:text-rose-400">
        Couldn't load members: {state.error}
      </p>
    );
  }
  if (members.length === 0) {
    return (
      <p className="rounded-lg border border-dashed border-stone-300 dark:border-slate-700 bg-stone-50/50 dark:bg-slate-950/30 px-4 py-3 text-xs text-stone-500 dark:text-slate-400">
        No active members yet. Add members from the main dashboard's Household
        panel before granting rewards.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {/* Member picker — single-select list with avatar + role + reward total */}
      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
          Pick a member
        </span>
        <div className="flex max-h-44 flex-col gap-1 overflow-y-auto rounded-lg border border-stone-200 dark:border-slate-800 bg-stone-50/40 dark:bg-slate-950/30 p-1">
          {members.map((m) => {
            const wallet = m.account.wallet;
            const base58 = wallet.toBase58();
            const role = roleFromAnchor(m.account.role) as Role | null;
            const isSelected = selectedWallet === base58;
            return (
              <button
                key={base58}
                type="button"
                onClick={() => setSelectedWallet(base58)}
                className={cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-left transition-colors",
                  isSelected
                    ? "bg-emerald-50 dark:bg-emerald-500/10 ring-1 ring-emerald-500/30"
                    : "hover:bg-stone-100 dark:hover:bg-slate-800/60"
                )}
              >
                <Avatar seed={base58} size="md" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-stone-800 dark:text-slate-100">
                    {role === "owner"
                      ? "Admin"
                      : role
                      ? `${role[0].toUpperCase()}${role.slice(1)}`
                      : "Member"}
                  </div>
                  <div className="truncate font-mono text-[10px] text-stone-500 dark:text-slate-500">
                    {base58.slice(0, 6)}…{base58.slice(-4)}
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-slate-600">
                    Points
                  </div>
                  <div className="text-sm font-semibold text-stone-800 dark:text-slate-100">
                    {m.account.rewardPoints.toString(10)}
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      </div>

      {/* Points + reason */}
      <div className="grid gap-2 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
            Points
          </span>
          <input
            type="number"
            min={1}
            step={1}
            inputMode="numeric"
            value={pointsInput}
            onChange={(e) => setPointsInput(e.target.value)}
            placeholder="e.g. 25"
            className={cn(
              "w-full rounded-lg border bg-white dark:bg-slate-950/60 px-3 py-2 font-mono text-sm text-stone-800 dark:text-slate-100 outline-none transition",
              "placeholder:text-stone-400 dark:placeholder:text-slate-600",
              "focus:border-emerald-500/70 focus:ring-1 focus:ring-emerald-500/40",
              pointsError
                ? "border-rose-500/70"
                : "border-stone-200 dark:border-slate-800"
            )}
          />
          {pointsError && (
            <p className="text-xs text-rose-600 dark:text-rose-400">
              {pointsError}
            </p>
          )}
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-slate-400">
            Reason
          </span>
          <input
            type="text"
            value={reasonInput}
            onChange={(e) => setReasonInput(e.target.value)}
            placeholder='e.g. "Found a coupon"'
            className={cn(
              "w-full rounded-lg border border-stone-200 dark:border-slate-800",
              "bg-white dark:bg-slate-950/60 px-3 py-2 text-sm text-stone-800 dark:text-slate-100 outline-none",
              "placeholder:text-stone-400 dark:placeholder:text-slate-600",
              "focus:border-emerald-500/70 focus:ring-1 focus:ring-emerald-500/40"
            )}
          />
        </div>
      </div>

      <div className="flex items-center gap-2">
        <Button
          onClick={handleSubmit}
          loading={tx.pending}
          disabled={!canSubmit}
        >
          Give points
        </Button>
        <span className="text-[11px] text-stone-500 dark:text-slate-500">
          🔒 Only a hash of the reason goes on-chain.
        </span>
      </div>

      <ResultBanner
        pending={tx.pending}
        signature={tx.signature}
        error={tx.error}
        onDismiss={tx.reset}
      />
    </div>
  );
}

function MemberSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      <div className="h-3 w-24 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
      {Array.from({ length: 3 }).map((_, i) => (
        <div key={i} className="flex items-center gap-3 rounded-md px-3 py-2">
          <div className="h-8 w-8 animate-pulse rounded-lg bg-stone-200 dark:bg-slate-800" />
          <div className="h-3 flex-1 animate-pulse rounded bg-stone-200 dark:bg-slate-800" />
        </div>
      ))}
    </div>
  );
}
