// Stocksie instruction builders (plan 007 v0.5) — pure Rust, no Anchor client.
//
// Everything here is derived from app/idl/stocksie.json (the ground truth):
// instruction discriminators are copied verbatim from the IDL (they are the
// first 8 bytes of sha256("global:<name>")), account ORDER and
// writable/signer flags match the IDL exactly, and args are borsh-encoded by
// hand. The native E2E test (tests/e2e_localnet.rs) executes all four
// instructions against the actual deployed program on localnet, so any
// encoding drift fails loudly.

use crate::tx::ProgramIx;
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const STOCKSIE_PROGRAM_ID: &str = "At2vd5Mqd9xcHRFAZin1VHb6upEsF1GCXoFG19UHsQoj";

pub fn program_id() -> Pubkey {
    Pubkey::from_str(STOCKSIE_PROGRAM_ID).expect("constant parses")
}

/// household PDA: ["household", owner]
pub fn household_pda(owner: &Pubkey, program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"household", owner.as_ref()], program)
}

/// member PDA: ["member", household, wallet]
pub fn member_pda(household: &Pubkey, wallet: &Pubkey, program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"member", household.as_ref(), wallet.as_ref()], program).0
}

/// request PDA: ["purchase", household, request_id LE u64]
pub fn purchase_pda(household: &Pubkey, request_id: u64, program: &Pubkey) -> Pubkey {
    let id_le = request_id.to_le_bytes();
    Pubkey::find_program_address(&[b"purchase", household.as_ref(), &id_le], program).0
}

/// blake3 hash of arbitrary client text — mirrors lib/hashes.ts `toHash32`.
/// The raw text never lands on-chain (privacy by design).
pub fn text_hash(text: &str) -> [u8; 32] {
    use blake3::Hasher;
    let mut hasher = Hasher::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

fn ix(program: &Pubkey, disc: [u8; 8], accounts: Vec<(Pubkey, bool, bool)>, args: &[u8]) -> ProgramIx {
    let mut data = Vec::with_capacity(8 + args.len());
    data.extend_from_slice(&disc);
    data.extend_from_slice(args);
    ProgramIx { program: *program, accounts, data }
}

// discriminators copied from app/idl/stocksie.json
const DISC_INITIALIZE_HOUSEHOLD: [u8; 8] = [105, 174, 153, 81, 23, 131, 169, 140];
const DISC_DEPOSIT_FUNDS: [u8; 8] = [202, 39, 52, 211, 53, 20, 250, 88];
const DISC_CREATE_PURCHASE_REQUEST: [u8; 8] = [154, 78, 58, 3, 144, 59, 248, 157];
const DISC_APPROVE_PURCHASE_REQUEST: [u8; 8] = [120, 116, 66, 219, 241, 134, 110, 193];
const DISC_ADD_MEMBER: [u8; 8] = [13, 116, 123, 130, 126, 198, 57, 34];
const DISC_CONFIRM_RESTOCK: [u8; 8] = [168, 30, 4, 173, 92, 59, 24, 106];
const DISC_REIMBURSE_BUYER: [u8; 8] = [194, 20, 70, 81, 246, 45, 24, 35];
const DISC_REJECT_PURCHASE_REQUEST: [u8; 8] = [159, 23, 80, 0, 1, 144, 72, 23];
const DISC_CLOSE_PURCHASE_REQUEST: [u8; 8] = [0, 203, 46, 255, 73, 162, 24, 115];
const DISC_WITHDRAW_FUNDS: [u8; 8] = [241, 36, 29, 111, 208, 31, 104, 217];
const DISC_AWARD_REWARD: [u8; 8] = [223, 205, 213, 32, 234, 132, 85, 29];
const DISC_REMOVE_MEMBER: [u8; 8] = [171, 57, 231, 150, 167, 128, 18, 55];
const DISC_SET_ROLE: [u8; 8] = [77, 78, 62, 233, 192, 61, 199, 190];

/// initialize_household(household(w), owner_member(w), owner(w,s), system)
/// args: name_hash [u8;32]
pub fn initialize_household(
    owner: &Pubkey,
    name_hash: [u8; 32],
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let owner_member = member_pda(&household, owner, program);
    ix(
        program,
        DISC_INITIALIZE_HOUSEHOLD,
        vec![
            (household, true, false),
            (owner_member, true, false),
            (*owner, true, true),
            (crate::tx::system_program(), false, false),
        ],
        &name_hash,
    )
}

/// deposit_funds(household(w), depositor_member(ro), depositor(w,s), system)
/// args: lamports u64
pub fn deposit_funds(depositor: &Pubkey, lamports: u64, program: &Pubkey) -> ProgramIx {
    let household = household_pda(depositor, program).0;
    let depositor_member = member_pda(&household, depositor, program);
    ix(
        program,
        DISC_DEPOSIT_FUNDS,
        vec![
            (household, true, false),
            (depositor_member, false, false),
            (*depositor, true, true),
            (crate::tx::system_program(), false, false),
        ],
        &lamports.to_le_bytes(),
    )
}

/// create_purchase_request(household(w), caller_member(w), request(w),
/// buyer_member(w), caller(w,s), system)
/// args: amount_lamports u64, item_hash[32], unit_cost_hash[32], buyer pubkey
pub fn create_purchase_request(
    caller: &Pubkey,
    buyer: &Pubkey,
    request_counter: u64,
    amount_lamports: u64,
    item_hash: [u8; 32],
    unit_cost_hash: [u8; 32],
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(caller, program).0;
    let caller_member = member_pda(&household, caller, program);
    let request = purchase_pda(&household, request_counter, program);
    let buyer_member = member_pda(&household, buyer, program);
    let mut args = Vec::with_capacity(8 + 32 + 32 + 32);
    args.extend_from_slice(&amount_lamports.to_le_bytes());
    args.extend_from_slice(&item_hash);
    args.extend_from_slice(&unit_cost_hash);
    args.extend_from_slice(buyer.as_ref());
    ix(
        program,
        DISC_CREATE_PURCHASE_REQUEST,
        vec![
            (household, true, false),
            (caller_member, true, false),
            (request, true, false),
            (buyer_member, true, false),
            (*caller, true, true),
            (crate::tx::system_program(), false, false),
        ],
        &args,
    )
}

/// add_member(household(w), caller_member(ro), new_member(w), caller(w,s),
/// system) args: new_member_wallet pubkey, role u8 (borsh unit-enum tag:
/// Owner=0, Parent=1, Child=2, Guest=3)
pub fn add_member(
    caller: &Pubkey,
    new_member: &Pubkey,
    role_tag: u8,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(caller, program).0;
    let caller_member = member_pda(&household, caller, program);
    let new_member_pda = member_pda(&household, new_member, program);
    let mut args = Vec::with_capacity(33);
    args.extend_from_slice(new_member.as_ref());
    args.push(role_tag);
    ix(
        program,
        DISC_ADD_MEMBER,
        vec![
            (household, true, false),
            (caller_member, false, false),
            (new_member_pda, true, false),
            (*caller, true, true),
            (crate::tx::system_program(), false, false),
        ],
        &args,
    )
}

/// confirm_restock(household(w), buyer_member(w), request(w), buyer(s))
/// args: unit_cost_hash [u8;32] — signed BY the buyer. `owner` seeds the
/// household PDA (the buyer is just a member of that household).
pub fn confirm_restock(
    owner: &Pubkey,
    buyer: &Pubkey,
    request_id: u64,
    unit_cost_hash: [u8; 32],
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let buyer_member = member_pda(&household, buyer, program);
    let request = purchase_pda(&household, request_id, program);
    ix(
        program,
        DISC_CONFIRM_RESTOCK,
        vec![
            (household, true, false),
            (buyer_member, true, false),
            (request, true, false),
            (*buyer, false, true),
        ],
        &unit_cost_hash,
    )
}

/// reimburse_buyer(household(w), caller_member(ro), request(w),
/// buyer_member(w), buyer(w), caller(s)) args: lamports u64 — Owner/Parent
/// only. `owner` seeds the household; `caller` signs (may be a Parent).
pub fn reimburse_buyer(
    owner: &Pubkey,
    caller: &Pubkey,
    buyer: &Pubkey,
    request_id: u64,
    lamports: u64,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let caller_member = member_pda(&household, caller, program);
    let request = purchase_pda(&household, request_id, program);
    let buyer_member = member_pda(&household, buyer, program);
    ix(
        program,
        DISC_REIMBURSE_BUYER,
        vec![
            (household, true, false),
            (caller_member, false, false),
            (request, true, false),
            (buyer_member, true, false),
            (*buyer, true, false),
            (*caller, false, true),
        ],
        &lamports.to_le_bytes(),
    )
}

/// reject_purchase_request(household(ro), caller_member(ro), request(w),
/// caller(s)) args: reason_hash [u8;32] — Owner/Parent only.
pub fn reject_purchase_request(
    owner: &Pubkey,
    caller: &Pubkey,
    request_id: u64,
    reason_hash: [u8; 32],
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let caller_member = member_pda(&household, caller, program);
    let request = purchase_pda(&household, request_id, program);
    ix(
        program,
        DISC_REJECT_PURCHASE_REQUEST,
        vec![
            (household, false, false),
            (caller_member, false, false),
            (request, true, false),
            (*caller, false, true),
        ],
        &reason_hash,
    )
}

/// close_purchase_request(household(ro), caller_member(ro), request(w),
/// caller(w,s)) — reclaims the request's rent to the caller. Terminal states
/// only (reimbursed / rejected).
pub fn close_purchase_request(
    owner: &Pubkey,
    caller: &Pubkey,
    request_id: u64,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let caller_member = member_pda(&household, caller, program);
    let request = purchase_pda(&household, request_id, program);
    ix(
        program,
        DISC_CLOSE_PURCHASE_REQUEST,
        vec![
            (household, false, false),
            (caller_member, false, false),
            (request, true, false),
            (*caller, true, true),
        ],
        &[],
    )
}

/// withdraw_funds(household(w), owner_member(ro), owner(w,s), system)
/// args: lamports u64 — the admin drains the vault (money always goes to the
/// admin's own account).
pub fn withdraw_funds(owner: &Pubkey, lamports: u64, program: &Pubkey) -> ProgramIx {
    let household = household_pda(owner, program).0;
    let owner_member = member_pda(&household, owner, program);
    ix(
        program,
        DISC_WITHDRAW_FUNDS,
        vec![
            (household, true, false),
            (owner_member, false, false),
            (*owner, true, true),
            (crate::tx::system_program(), false, false),
        ],
        &lamports.to_le_bytes(),
    )
}

/// award_reward(household(w), caller_member(ro), target_member(w), caller(s))
/// args: member_wallet pubkey, points u64, reason_hash [u8;32] — Owner/Parent
/// only. `admin` seeds the household; `target` receives the points.
pub fn award_reward(
    admin: &Pubkey,
    caller: &Pubkey,
    target: &Pubkey,
    points: u64,
    reason_hash: [u8; 32],
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(admin, program).0;
    let caller_member = member_pda(&household, caller, program);
    let target_member = member_pda(&household, target, program);
    let mut args = Vec::with_capacity(32 + 8 + 32);
    args.extend_from_slice(target.as_ref());
    args.extend_from_slice(&points.to_le_bytes());
    args.extend_from_slice(&reason_hash);
    ix(
        program,
        DISC_AWARD_REWARD,
        vec![
            (household, true, false),
            (caller_member, false, false),
            (target_member, true, false),
            (*caller, false, true),
        ],
        &args,
    )
}

/// remove_member(household(w), caller_member(ro), target_member(w),
/// caller(w,s)) args: member_wallet pubkey — Owner only.
pub fn remove_member(
    admin: &Pubkey,
    caller: &Pubkey,
    target: &Pubkey,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(admin, program).0;
    let caller_member = member_pda(&household, caller, program);
    let target_member = member_pda(&household, target, program);
    ix(
        program,
        DISC_REMOVE_MEMBER,
        vec![
            (household, true, false),
            (caller_member, false, false),
            (target_member, true, false),
            (*caller, true, true),
        ],
        target.as_ref(),
    )
}

/// set_role(household(ro), caller_member(ro), target_member(w), caller(s))
/// args: new_role u8 (borsh unit-enum tag), member_wallet pubkey — Owner only.
pub fn set_role(
    admin: &Pubkey,
    caller: &Pubkey,
    target: &Pubkey,
    role_tag: u8,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(admin, program).0;
    let caller_member = member_pda(&household, caller, program);
    let target_member = member_pda(&household, target, program);
    let mut args = Vec::with_capacity(1 + 32);
    args.push(role_tag);
    args.extend_from_slice(target.as_ref());
    ix(
        program,
        DISC_SET_ROLE,
        vec![
            (household, false, false),
            (caller_member, false, false),
            (target_member, true, false),
            (*caller, false, true),
        ],
        &args,
    )
}

/// approve_purchase_request(household(ro), caller_member(ro), request(w),
/// caller(ro,s))
pub fn approve_purchase_request(
    caller: &Pubkey,
    request_id: u64,
    program: &Pubkey,
) -> ProgramIx {
    let household = household_pda(caller, program).0;
    let caller_member = member_pda(&household, caller, program);
    let request = purchase_pda(&household, request_id, program);
    ix(
        program,
        DISC_APPROVE_PURCHASE_REQUEST,
        vec![
            (household, false, false),
            (caller_member, false, false),
            (request, true, false),
            (*caller, false, true),
        ],
        &[],
    )
}
