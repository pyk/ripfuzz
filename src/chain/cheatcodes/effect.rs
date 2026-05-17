//! Centralized effect system for cheatcodes.
//!
//! Every cheatcode produces a list of `CheatcodeEffect` variants.  A single
//! `apply_effect` function is the only place that mutates `ctx.block`,
//! `ctx.journal_mut()`, and `inspector.state`.

use revm::{
    context::{BlockEnv, ContextSetters},
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    database::InMemoryDB,
    primitives::{Address, Bytes, U256},
};

use crate::chain::cheatcodes::{CheatcodeState, DealRecord, PrankState, StartPrankState};

/// Minimal trait to mutate `chain_id` on generic EVM contexts.
///
/// Only `chainId` needs this because it mutates `cfg.chain_id` live during a
/// call so the `CHAINID` opcode sees the new value immediately.
pub trait CfgMut {
    fn set_chain_id(&mut self, chain_id: u64);
}

impl<BLOCK, TX, DB, JOURNAL, CHAIN, LOCAL, SPEC> CfgMut
    for revm::context::Context<BLOCK, TX, revm::context::CfgEnv<SPEC>, DB, JOURNAL, CHAIN, LOCAL>
where
    DB: revm::Database,
    JOURNAL: revm::context_interface::JournalTr<Database = DB>,
    LOCAL: revm::context_interface::LocalContextTr,
{
    fn set_chain_id(&mut self, chain_id: u64) {
        self.cfg.chain_id = chain_id;
    }
}

/// What a cheatcode wants to change in the EVM or inspector state.
#[derive(Clone, Debug, PartialEq)]
pub enum CheatcodeEffect {
    // --- EVM context mutations ---
    // apply_effect auto-persists these into state.block for cross-call
    // persistence.
    SetBlockTimestamp(U256),
    SetBlockNumber(U256),
    SetBaseFee(u64),
    SetBeneficiary(Address),
    SetPrevrandao([u8; 32]),

    // --- DB mutations ---
    SetAccountBalance(Address, U256),
    SetAccountCode(Address, Bytes),
    SetAccountNonce(Address, u64),
    SetStorage(Address, U256, U256),

    // --- Inspector state mutations ---
    // These mutate state without touching ctx (either no ctx accessor
    // exists, or the mutation must survive across call boundaries).
    SetChainId(U256),
    SetPrank(PrankState),
    SetStartPrank(StartPrankState),
    ClearPrank,
    AddLabel(Address, String),
    SetFfiEnabled(bool),

    // --- Read effects (resolved by build_outcome) ---
    ReadNonce(Address),
    ReadBalance(Address),
    ReadStorage(Address, U256),
    GetLabel(Address),

    // --- Outcome effects ---
    Revert(String),
    Panic,
    ReturnU256(U256),
    ReturnBool(bool),
    ReturnBytes(Vec<u8>),

    // --- Special effects that need inspector state to resolve ---
    GetCode(String),
    FfiExec(Vec<String>),
}

/// Apply a single effect, mutating `ctx` and/or `state`.
///
/// Returns `Err(reason)` if the effect cannot be applied (e.g. FFI disabled).
pub fn apply_effect<CTX: ContextTr<Db = InMemoryDB> + ContextSetters<Block = BlockEnv> + CfgMut>(
    effect: &CheatcodeEffect,
    ctx: &mut CTX,
    state: &mut CheatcodeState,
) -> Result<(), String> {
    match effect {
        // --- EVM context mutations ---
        CheatcodeEffect::SetBlockTimestamp(v) => {
            let mut block = ctx.block().clone();
            block.timestamp = *v;
            ctx.set_block(block);
            state.block.timestamp = Some(*v);
        }
        CheatcodeEffect::SetBlockNumber(v) => {
            let mut block = ctx.block().clone();
            block.number = *v;
            ctx.set_block(block);
            state.block.number = Some(*v);
        }
        CheatcodeEffect::SetBaseFee(v) => {
            let mut block = ctx.block().clone();
            block.basefee = *v;
            ctx.set_block(block);
            state.block.basefee = Some(U256::from(*v));
        }
        CheatcodeEffect::SetBeneficiary(addr) => {
            let mut block = ctx.block().clone();
            block.beneficiary = *addr;
            ctx.set_block(block);
            state.block.beneficiary = Some(*addr);
        }
        CheatcodeEffect::SetPrevrandao(bytes) => {
            let mut block = ctx.block().clone();
            block.prevrandao = Some(revm::primitives::FixedBytes::from(*bytes));
            ctx.set_block(block);
            state.block.prevrandao = Some(*bytes);
        }

        // --- DB mutations ---
        CheatcodeEffect::SetAccountBalance(addr, v) => {
            let old_balance = ctx
                .journal_mut()
                .load_account(*addr)
                .ok()
                .map(|s| s.data.info.balance)
                .unwrap_or(U256::ZERO);
            let mut acc = ctx
                .journal_mut()
                .load_account_mut(*addr)
                .map_err(|_| "account load failed")?
                .data;
            acc.set_balance(*v);
            state.eth_deals.push(DealRecord {
                address: *addr,
                old_balance,
                new_balance: *v,
            });
        }
        CheatcodeEffect::SetAccountCode(addr, code) => {
            if ctx.journal().precompile_addresses().contains(addr) {
                return Err("cannot etch precompile address".into());
            }
            ctx.journal_mut()
                .load_account(*addr)
                .map_err(|_| "account load failed")?;
            let bytecode = revm::bytecode::Bytecode::new_raw_checked(code.clone())
                .map_err(|e| format!("failed to create bytecode: {e}"))?;
            ctx.journal_mut().set_code(*addr, bytecode);
        }
        CheatcodeEffect::SetAccountNonce(addr, nonce) => {
            let current = ctx
                .journal_mut()
                .load_account(*addr)
                .ok()
                .map(|s| s.data.info.nonce)
                .unwrap_or(0);
            if *nonce < current {
                return Err(format!(
                    "new nonce ({nonce}) must be strictly equal to or higher than the \
                     account's current nonce ({current})"
                ));
            }
            let mut acc = ctx
                .journal_mut()
                .load_account_mut(*addr)
                .map_err(|_| "account load failed")?
                .data;
            acc.set_nonce(*nonce);
            state
                .nonce_changes
                .push(crate::chain::cheatcodes::NonceRecord {
                    address: *addr,
                    old_nonce: current,
                    new_nonce: *nonce,
                });
        }
        CheatcodeEffect::SetStorage(addr, slot, value) => {
            if ctx.journal().precompile_addresses().contains(addr) {
                return Err("store: cannot write to precompile".into());
            }
            // Ensure account is loaded into the journal before sstore.
            ctx.journal_mut()
                .load_account(*addr)
                .map_err(|_| "account load failed")?;
            ctx.journal_mut()
                .sstore(*addr, *slot, *value)
                .map_err(|e| format!("failed to store storage slot: {e:?}"))?;
        }

        // --- Inspector state mutations ---
        CheatcodeEffect::SetChainId(v) => {
            state.block.chain_id = Some(*v);
            // Also update the live EVM context so the CHAINID opcode sees the
            // new value for the remainder of the current call.
            ctx.set_chain_id(u64::try_from(*v).unwrap_or(u64::MAX));
        }
        CheatcodeEffect::SetPrank(p) => {
            // Foundry semantics: a prank can be overwritten only if it was
            // already used.  vm.prank cannot overwrite an ongoing startPrank.
            if let Some(ref active) = state.prank.active
                && !active.used
            {
                return Err(
                    "prank(address) cannot be called when a prank is already active".into(),
                );
            }
            if state.prank.start.is_some() {
                return Err(
                    "prank(address) cannot be called when a startPrank is already active".into(),
                );
            }
            state.prank.active = Some(*p);
        }
        CheatcodeEffect::SetStartPrank(p) => {
            // Foundry semantics: startPrank can overwrite a used startPrank,
            // but not an unused prank or an unused startPrank.
            if let Some(ref active) = state.prank.active
                && !active.used
            {
                return Err(
                    "startPrank(address) cannot be called when a prank is already active".into(),
                );
            }
            if let Some(ref start) = state.prank.start
                && !start.used
            {
                return Err(
                    "startPrank(address) cannot be called when a startPrank is already active"
                        .into(),
                );
            }
            state.prank.start = Some(*p);
        }
        CheatcodeEffect::ClearPrank => {
            state.prank.active = None;
            state.prank.start = None;
        }
        CheatcodeEffect::AddLabel(addr, name) => {
            state.labels.insert(*addr, name.clone());
        }
        CheatcodeEffect::SetFfiEnabled(v) => state.ffi_enabled = *v,

        // --- Read / outcome / special effects do not mutate state ---
        CheatcodeEffect::ReadNonce(_)
        | CheatcodeEffect::ReadBalance(_)
        | CheatcodeEffect::ReadStorage(_, _)
        | CheatcodeEffect::GetLabel(_)
        | CheatcodeEffect::Revert(_)
        | CheatcodeEffect::Panic
        | CheatcodeEffect::ReturnU256(_)
        | CheatcodeEffect::ReturnBool(_)
        | CheatcodeEffect::ReturnBytes(_)
        | CheatcodeEffect::GetCode(_)
        | CheatcodeEffect::FfiExec(_) => {}
    }
    Ok(())
}
