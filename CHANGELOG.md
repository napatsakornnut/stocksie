# Changelog

All notable changes to Stocksie are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — MVP

The initial household-coordination MVP: 14 instructions across five groups, 12
events forming the audit trail, and a 75-test LiteSVM suite. All on-chain state
is privacy-preserving — no `String` field on any account or event; all
human-readable concepts are reduced to `[u8; 32]` blake3 hashes.

### Added — Program (`programs/stocksie/`)

- **Household lifecycle** (Feature 3.3 — access-controlled state transitions):

  - `initialize_household(name_hash)` — creates the `Household` PDA (also the
    shared SOL vault) and the owner's `Member` PDA. Owner role is set
    exclusively here; `member_count = 1`.
  - `add_member(new_member_wallet, role)` — Owner onboards a wallet under a
    role. Rejects `role == Owner` and enforces the `MAX_MEMBERS` (16) cap.
  - `remove_member(member_wallet)` — Owner closes a membership via
    `close = caller` (rent refunded, PDA wiped, re-addable via fresh `init`).
    Rejects removing the Owner.
  - `set_role(new_role, member_wallet)` — Owner changes a member's role.
    Rejects promotion to `Owner`.

- **Vault / funds** (Feature 2.4 / 3.2 — the shared household SOL vault):

  - `deposit_funds(lamports)` — top up the vault. Any active member (Guest
    included) may deposit.
  - `withdraw_funds(lamports)` — Owner-only emergency drain. The destination is
    fixed to the recorded Owner signer; there is no arbitrary `to` field.

- **Purchase lifecycle** (Features 2.1, 2.2, 3.3 — the shared shopping list as a
  strict state machine):

  - `create_purchase_request(amount_lamports, item_hash, unit_cost_hash, buyer)`
    — the "Last-One Tap" entry point. Range-checks the amount
    (`MIN_REQUEST_LAMPORTS … MAX_REIMBURSEMENT_LAMPORTS`), assigns a monotonic
    `request_id`, rewards the reporter (+10). `Pending` state.
  - `approve_purchase_request()` — `Pending → Approved`. Owner/Parent only,
    cannot be the buyer (`SelfApprovalForbidden`).
  - `reject_purchase_request(reason_hash)` — `Pending | Approved → Rejected`
    (terminal). `reason_hash` may be `[0; 32]` for "no reason".
  - `confirm_restock(unit_cost_hash)` — `Approved → Restocked`. Recorded buyer
    only. Overwrites `unit_cost_hash` with the actual-purchase snapshot.
    Rewards the buyer (+25).
  - `close_purchase_request()` — reclaims rent from a terminal request.
    `close = caller`; caller must be the Owner or the request's buyer.

- **Reimbursement** (Features 2.2, 2.4, 3.2, 3.4 — the trust-critical vault →
  buyer SOL transfer):

  - `reimburse_buyer(lamports)` — `Restocked → Reimbursed`. Caller is the
    approver (Owner/Parent), not the buyer. Direct lamport move from the
    program-owned vault PDA to the recorded buyer. One-shot, ceiling-enforced
    (`lamports ≤ amount_lamports`). Rewards the buyer (+15) for the full run.

- **Rewards** (Feature 2.5 — gamification audit stream):

  - `award_reward(member_wallet, points, reason_hash)` — Owner/Parent manually
    grants points to any active member. `reason_hash` is a client-computed
    blake3 digest of the off-chain reason.
  - `reward_summary()` — read-only score fetch. Emits a sentinel
    `RewardEarned` (`points == 0`, `reason_hash == [0; 32]`) so a score-fetch is
    distinguishable from a real grant.

- **Events** — 12 `#[event]` structs forming the tamper-proof audit trail:
  `HouseholdCreated`, `MemberAdded`, `MemberRemoved`, `RoleChanged`,
  `FundsDeposited`, `FundsWithdrawn`, `PurchaseCreated`, `PurchaseApproved`,
  `PurchaseRejected`, `Restocked`, `Reimbursed`, `RewardEarned`. Every payload
  is a permitted shape (pubkeys, lamports/points, slots, `[u8; 32]` hashes).

- **State model** — three `#[account]` structs with `#[derive(InitSpace)]`:

  - `Household` (`INIT_SPACE = 101`, on-chain 109 B) — family record and the
    shared SOL vault at one PDA. Mirrors `vault_balance` to `account.lamports()`.
  - `Member` (`INIT_SPACE = 83`, on-chain 91 B) — per-wallet membership record.
  - `PurchaseRequest` (`INIT_SPACE = 218`, on-chain 226 B) — shared-list entry
    with the strict lifecycle and per-stage reward ledger.

- **Cross-cutting guarantees**:
  - Checked arithmetic everywhere (`checked_add` / `checked_sub`); release
    profile sets `overflow-checks = true`.
  - `init` only — never `init_if_needed` (reinitialization-proof).
  - Canonical bumps stored at `init` and reused for CPI signing.
  - `has_one = household` on every cross-account reference.
  - Privacy invariant: no `String` field on any account or event.

### Added — Test suite (`programs/stocksie/tests/` + inline `#[cfg(test)]`)

- 24 pure unit tests (`cargo test -p stocksie --lib`) — state-machine guards,
  `Role::can_*()` predicates, reward accumulators, reward-reason hashing,
  close-authority and authority-gate sanity. The 24th is the `test_id` test
  auto-generated by Anchor's `declare_id!` macro.
- 51 LiteSVM integration tests (`cargo test -p stocksie --tests`) across nine
  files, exercising the real compiled `stocksie.so` in-process:
  - `test_household.rs` — init, add_member, remove_member rent refund (4).
  - `test_lifecycle.rs` — full happy path, partial reimbursement, rejection
    moves no funds (3).
  - `test_permissions.rs` — every role gate, self-approval, non-buyer restock,
    owner-protection, and forbidden state transition (15).
  - `test_reimburse.rs` — every reimbursement edge case: wrong-state, over
    ceiling, zero, double, insufficient vault (7).
  - `test_rewards.rs` — award, summary sentinel, zero-reject, overflow (4).
  - `test_security.rs` — Solana attack-category defenses: fake account,
    unsigned call, double init, cross-household isolation, type cosplay,
    close-then-revive, has_one, canonical bump, pre-funded PDA, cross-household
    reimbursement (10).
  - `test_invariants.rs` — cross-cutting reconciliation: treasury, reward-sum,
    membership-set, no-self-approval-in-stream, lifecycle monotonicity (5).
  - `test_privacy_invariant.rs` — the `no_string_fields_on_chain` source grep
    and its scanner-coverage companion (2).
  - `test_space.rs` — locks `INIT_SPACE` for all three accounts against drift (1).
- Shared harness in `tests/helpers/mod.rs`: `setup_svm`, `derive_household` /
  `derive_member` / `derive_request`, `build_ix`, `send`, `account_of`,
  `balance_of`, `emitted_events_of`, `assert_error_code`, plus the
  `reach_restocked` / `reach_reimbursed` lifecycle helpers.

### Added — Documentation

- `README.md` — overview, the five core features, "why Solana?", the
  architecture diagram, the verified quickstart, and the project layout.
- `docs/ARCHITECTURE.md` — tech stack, version matrix, project layout, the
  five-layer instruction shape, PDA derivation, environment-quirk appendix.
- `docs/ACCOUNTS.md` — the three account structs, field-by-field rationale,
  space/rent budget, the state machine, and the forbidden-transitions table.
- `docs/INSTRUCTIONS.md` — every instruction's accounts, args, effect, events,
  and errors; the authority model; the instruction → event matrix; the
  cross-cutting rules; a client example.
- `docs/SECURITY.md` — the vulnerability matrix (each Solana attack category →
  defense → verifying test), program-specific invariants, arithmetic-safety
  policy, the privacy invariant, CPI/closure safety, and the pre-deployment
  checklist.
- `docs/TESTING.md` — the testing pyramid, the verified run commands, the
  harness pattern, the full test matrix (actual names), the negative-test and
  event-assertion patterns, the CI sketch, and where to add a new test.
- `docs/PRIVACY.md` — the canonical on-chain/off-chain boundary contract, the
  "Last-One Tap" walkthrough, off-chain responsibilities, the tamper-evidence
  story, and an honest gap list.
- `docs/ROADMAP.md` — MVP scope (shipped), the ~4-week horizon, the later
  horizon, and the explicit out-of-scope list.
- `plan/` — the internal design rationale (concept, architecture, account
  model, instructions, state machine, events, security, testing, build phases,
  docs plan). Intentionally kept in the repo as the design source of truth.

### Tooling / build

- Anchor `1.0.2` (`anchor-lang` crate + `anchor-cli`), Agave Solana CLI `3.1.10`.
- `blake3 = "1"` for privacy-preserving reward-reason and item/receipt hashing
  (project standard; preferred over SHA-256).
- `litesvm = "0.10.0"` + `solana-* 3.0.x` dev-deps for the in-process test VM.
- Workspace release profile: `overflow-checks = true`, `lto = "fat"`,
  `codegen-units = 1`.
- Program ID: `At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj`.

### Security

- No known vulnerabilities in the shipped surface. Every claim in
  `docs/SECURITY.md` is backed by a named test or explicitly marked "code
  review". Two dedicated tests (`aliased_vault_debit_rejected`,
  `overflow_returns_error`) are flagged as low-effort follow-ups in
  `docs/SECURITY.md` §11 — the defenses exist; only those two tests do not.

[Unreleased]: https://github.com/napatsakornnut/stocksie/compare/HEAD
