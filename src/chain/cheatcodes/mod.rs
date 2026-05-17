//! Cheatcode extension point for Foundry-compatible precompiles.
//!
//! Each cheatcode category lives in its own file and exports a struct
//! implementing the [`Cheatcode`] trait.  The dispatch table in this module
//! is the single place where new cheatcodes are registered.

use std::collections::HashMap;
use std::process::Command;

use revm::{
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    database::InMemoryDB,
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, U256},
};

pub use deal::DealRecord;
pub use nonce::NonceRecord;

use crate::chain::cheatcodes::effect::CheatcodeEffect;

pub mod account;
pub mod assert;
pub mod chain_id;
pub mod coinbase;
pub mod deal;
pub mod difficulty;
pub mod effect;
pub mod etch;
pub mod fee;
pub mod ffi;
pub mod label;
pub mod nonce;
pub mod prank;
pub mod prevrandao;
pub mod roll;
pub mod string;
pub mod wallet;
pub mod warp;

/// Foundry cheatcode VM contract address.
pub const VM_ADDRESS: Address = Address::new([
    0x71, 0x09, 0x70, 0x9e, 0xcf, 0xa9, 0x1a, 0x80, 0x62, 0x6f, 0xf3, 0x98, 0x9d, 0x68, 0xf6, 0x7f,
    0x5b, 0x1d, 0xd1, 0x2d,
]);

// ---------------------------------------------------------------------------
//  Trait every cheatcode struct must implement.
// ---------------------------------------------------------------------------

pub trait Cheatcode {
    const SELECTOR: [u8; 4];
    type Args;

    fn decode(input: &Bytes) -> Option<Self::Args>;
    fn effects(args: Self::Args) -> Vec<CheatcodeEffect>;
}

fn dispatch<C: Cheatcode>(input: &Bytes) -> Option<Vec<CheatcodeEffect>> {
    let args = C::decode(input)?;
    Some(C::effects(args))
}

pub(crate) fn dispatch_effects(sel: [u8; 4], input: &Bytes) -> Option<Vec<CheatcodeEffect>> {
    match sel {
        // Block / state manipulation
        warp::Warp::SELECTOR => dispatch::<warp::Warp>(input),
        roll::Roll::SELECTOR => dispatch::<roll::Roll>(input),
        fee::Fee::SELECTOR => dispatch::<fee::Fee>(input),
        coinbase::Coinbase::SELECTOR => dispatch::<coinbase::Coinbase>(input),
        prevrandao::Prevrandao::SELECTOR => dispatch::<prevrandao::Prevrandao>(input),
        chain_id::ChainId::SELECTOR => dispatch::<chain_id::ChainId>(input),
        difficulty::Difficulty::SELECTOR => dispatch::<difficulty::Difficulty>(input),

        // Account manipulation
        deal::Deal::SELECTOR => dispatch::<deal::Deal>(input),
        etch::Etch::SELECTOR => dispatch::<etch::Etch>(input),
        nonce::SetNonce::SELECTOR => dispatch::<nonce::SetNonce>(input),
        nonce::GetNonce::SELECTOR => dispatch::<nonce::GetNonce>(input),
        account::Load::SELECTOR => dispatch::<account::Load>(input),
        account::Store::SELECTOR => dispatch::<account::Store>(input),

        // Prank
        prank::Prank::SELECTOR => dispatch::<prank::Prank>(input),
        prank::PrankHere::SELECTOR => dispatch::<prank::PrankHere>(input),
        prank::StartPrank::SELECTOR => dispatch::<prank::StartPrank>(input),
        prank::StopPrank::SELECTOR => dispatch::<prank::StopPrank>(input),

        // Label
        label::Label::SELECTOR => dispatch::<label::Label>(input),

        // Assertions
        assert::AssertTrue::SELECTOR => dispatch::<assert::AssertTrue>(input),
        assert::AssertFalse::SELECTOR => dispatch::<assert::AssertFalse>(input),
        assert::AssertEqBool::SELECTOR => dispatch::<assert::AssertEqBool>(input),
        assert::AssertEqUint::SELECTOR => dispatch::<assert::AssertEqUint>(input),
        assert::AssertEqInt::SELECTOR => dispatch::<assert::AssertEqInt>(input),
        assert::AssertEqAddress::SELECTOR => dispatch::<assert::AssertEqAddress>(input),
        assert::AssertEqBytes32::SELECTOR => dispatch::<assert::AssertEqBytes32>(input),
        assert::AssertEqString::SELECTOR => dispatch::<assert::AssertEqString>(input),
        assert::AssertEqBytes::SELECTOR => dispatch::<assert::AssertEqBytes>(input),
        assert::AssertNotEqBool::SELECTOR => dispatch::<assert::AssertNotEqBool>(input),
        assert::AssertNotEqUint::SELECTOR => dispatch::<assert::AssertNotEqUint>(input),
        assert::AssertNotEqInt::SELECTOR => dispatch::<assert::AssertNotEqInt>(input),
        assert::AssertNotEqAddress::SELECTOR => dispatch::<assert::AssertNotEqAddress>(input),
        assert::AssertNotEqBytes32::SELECTOR => dispatch::<assert::AssertNotEqBytes32>(input),
        assert::AssertNotEqString::SELECTOR => dispatch::<assert::AssertNotEqString>(input),
        assert::AssertNotEqBytes::SELECTOR => dispatch::<assert::AssertNotEqBytes>(input),
        assert::AssertLtUint::SELECTOR => dispatch::<assert::AssertLtUint>(input),
        assert::AssertLtInt::SELECTOR => dispatch::<assert::AssertLtInt>(input),
        assert::AssertLeUint::SELECTOR => dispatch::<assert::AssertLeUint>(input),
        assert::AssertLeInt::SELECTOR => dispatch::<assert::AssertLeInt>(input),
        assert::AssertGtUint::SELECTOR => dispatch::<assert::AssertGtUint>(input),
        assert::AssertGtInt::SELECTOR => dispatch::<assert::AssertGtInt>(input),
        assert::AssertGeUint::SELECTOR => dispatch::<assert::AssertGeUint>(input),
        assert::AssertGeInt::SELECTOR => dispatch::<assert::AssertGeInt>(input),

        // String / type conversion
        string::ToStringAddress::SELECTOR => dispatch::<string::ToStringAddress>(input),
        string::ToStringBool::SELECTOR => dispatch::<string::ToStringBool>(input),
        string::ToStringUint::SELECTOR => dispatch::<string::ToStringUint>(input),
        string::ToStringInt::SELECTOR => dispatch::<string::ToStringInt>(input),
        string::ToStringBytes32::SELECTOR => dispatch::<string::ToStringBytes32>(input),
        string::ToStringBytes::SELECTOR => dispatch::<string::ToStringBytes>(input),
        string::ParseUint::SELECTOR => dispatch::<string::ParseUint>(input),
        string::ParseInt::SELECTOR => dispatch::<string::ParseInt>(input),
        string::ParseBool::SELECTOR => dispatch::<string::ParseBool>(input),
        string::ParseAddress::SELECTOR => dispatch::<string::ParseAddress>(input),
        string::ParseBytes::SELECTOR => dispatch::<string::ParseBytes>(input),
        string::ParseBytes32::SELECTOR => dispatch::<string::ParseBytes32>(input),
        string::GetCode::SELECTOR => dispatch::<string::GetCode>(input),

        // Wallet / crypto
        wallet::Addr::SELECTOR => dispatch::<wallet::Addr>(input),
        wallet::Sign::SELECTOR => dispatch::<wallet::Sign>(input),

        // FFI
        ffi::Ffi::SELECTOR => dispatch::<ffi::Ffi>(input),

        // Unknown VM call: silently drop.
        _ => Some(vec![]),
    }
}

// ---------------------------------------------------------------------------
//  State
// ---------------------------------------------------------------------------

/// Persistent block-context overrides set by cheatcodes.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockCheatState {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<U256>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<[u8; 32]>,
    pub chain_id: Option<U256>,
}

/// Persistent prank state set by cheatcodes.
#[derive(Clone, Debug, Default)]
pub struct PrankCheatState {
    pub active: Option<PrankState>,
    pub start: Option<StartPrankState>,
}

impl PrankCheatState {
    /// Return the caller that should be used for the top-level transaction.
    pub fn caller_for_top_level(&self) -> Option<Address> {
        self.start
            .as_ref()
            .map(|s| s.caller)
            .or_else(|| self.active.as_ref().map(|p| p.caller))
    }
}

/// State accumulated by cheatcodes during execution.
#[derive(Clone, Debug, Default)]
pub struct CheatcodeState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub ffi_enabled: bool,
    /// Contract name -> initcode bytes, populated from the artifact so
    /// `vm.getCode` can resolve contracts by name.
    pub compiled_contracts: HashMap<String, Bytes>,
    /// Rollback records for `vm.deal` (Foundry semantics).
    pub eth_deals: Vec<DealRecord>,
    /// Rollback records for `vm.setNonce`.
    pub nonce_changes: Vec<NonceRecord>,
}

impl CheatcodeState {
    /// Return all block-context overrides that should be applied before a call.
    pub fn block_overrides(&self) -> BlockOverrides {
        BlockOverrides {
            timestamp: self.block.timestamp,
            number: self.block.number,
            basefee: self.block.basefee.map(|f| u64::try_from(f).unwrap_or(0)),
            beneficiary: self.block.beneficiary,
            prevrandao: self
                .block
                .prevrandao
                .map(revm::primitives::FixedBytes::from),
            chain_id: self
                .block
                .chain_id
                .map(|id| u64::try_from(id).unwrap_or(u64::MAX)),
        }
    }
}

/// Block-context overrides produced from `CheatcodeState`.
#[derive(Clone, Debug, Default)]
pub struct BlockOverrides {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<u64>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<revm::primitives::FixedBytes<32>>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrankState {
    pub caller: Address,
    pub origin: Address,
    pub single_call: bool,
    /// Call depth of the frame that configured this prank.
    pub set_depth: u64,
    /// `true` for `prankHere` semantics: applies only to the very next
    /// direct child call from the frame where it was set.
    pub here: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartPrankState {
    pub caller: Address,
    pub origin: Address,
    /// Call depth at which this prank was set.  Only applies to calls
    /// deeper than this depth (Medusa semantics).
    pub set_depth: u64,
}

// ---------------------------------------------------------------------------
//  Outcome helpers
// ---------------------------------------------------------------------------

pub(crate) fn build_outcome<CTX: ContextTr<Db = InMemoryDB>>(
    effects: &[CheatcodeEffect],
    gas_limit: u64,
    ctx: &mut CTX,
    state: &CheatcodeState,
) -> CallOutcome {
    if let Some(outcome) = effects.iter().find_map(|effect| match effect {
        CheatcodeEffect::Revert(reason) => Some(revert_outcome(reason)),
        CheatcodeEffect::Panic => Some(panic_outcome()),
        CheatcodeEffect::ReturnU256(v) => Some(success_u256_outcome(*v, gas_limit)),
        CheatcodeEffect::ReturnBool(v) => Some(success_bool_outcome(*v, gas_limit)),
        CheatcodeEffect::ReturnBytes(bytes) => {
            Some(success_bytes_outcome(bytes.clone(), gas_limit))
        }
        CheatcodeEffect::ReadNonce(addr) => {
            let nonce = ctx
                .journal_mut()
                .load_account(*addr)
                .ok()
                .map(|s| s.data.info.nonce)
                .unwrap_or(0);
            Some(success_u256_outcome(U256::from(nonce), gas_limit))
        }
        CheatcodeEffect::ReadBalance(addr) => {
            let balance = ctx
                .journal_mut()
                .load_account(*addr)
                .ok()
                .map(|s| s.data.info.balance)
                .unwrap_or(U256::ZERO);
            Some(success_u256_outcome(balance, gas_limit))
        }
        CheatcodeEffect::ReadStorage(addr, slot) => {
            let value = match ctx.journal_mut().load_account_mut(*addr) {
                Ok(mut s) => s
                    .data
                    .sload(*slot, false)
                    .ok()
                    .map(|r| r.data.present_value)
                    .unwrap_or(U256::ZERO),
                Err(_) => U256::ZERO,
            };
            Some(success_bytes_outcome(value.to_be_bytes_vec(), gas_limit))
        }
        CheatcodeEffect::GetCode(name) => {
            let initcode = state
                .compiled_contracts
                .get(name)
                .cloned()
                .unwrap_or_default();
            Some(success_bytes_outcome(
                alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode(),
                gas_limit,
            ))
        }
        CheatcodeEffect::FfiExec(args) => {
            if !state.ffi_enabled {
                return Some(revert_outcome("ffi disabled: enable via config"));
            }
            if args.is_empty() {
                return Some(revert_outcome("ffi: no command provided"));
            }
            let mut it = args.iter();
            let Some(cmd) = it.next() else {
                return Some(revert_outcome("ffi: no command provided"));
            };
            let mut command = Command::new(cmd);
            for arg in it {
                command.arg(arg);
            }
            let output = match command.output() {
                Ok(v) => v,
                Err(_) => return Some(revert_outcome("ffi command failed")),
            };
            if !output.status.success() {
                return Some(revert_outcome("ffi command failed"));
            }
            let stdout_bytes = output.stdout;
            let stdout = String::from_utf8_lossy(&stdout_bytes);
            let trimmed = stdout.trim();
            let bytes = match trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
            {
                Some(hex_str) => hex::decode(hex_str).unwrap_or(stdout_bytes),
                None => stdout_bytes,
            };
            Some(success_bytes_outcome(
                alloy_dyn_abi::DynSolValue::Bytes(bytes).abi_encode(),
                gas_limit,
            ))
        }
        _ => None,
    }) {
        return outcome;
    }
    // Default: silent success.
    let mut outcome = dummy_success();
    outcome.result.gas = Gas::new(gas_limit);
    outcome
}

pub(crate) fn dummy_success() -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Stop,
            output: Bytes::new(),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub(crate) fn panic_outcome() -> CallOutcome {
    let mut encoded = vec![0x4e, 0x48, 0x7b, 0x71];
    encoded.extend_from_slice(&[0u8; 31]);
    encoded.push(0x01);
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from(encoded),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub(crate) fn revert_outcome(reason: &str) -> CallOutcome {
    let mut encoded = vec![0x08, 0xc3, 0x79, 0xa0];
    encoded.extend_from_slice(&alloy_dyn_abi::DynSolValue::String(reason.into()).abi_encode());
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from(encoded),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub(crate) fn success_u256_outcome(value: U256, gas_limit: u64) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(value.to_be_bytes_vec()),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub(crate) fn success_bool_outcome(value: bool, gas_limit: u64) -> CallOutcome {
    let mut output = vec![0u8; 32];
    if value {
        output[31] = 1;
    }
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(output),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub(crate) fn success_bytes_outcome(bytes: Vec<u8>, gas_limit: u64) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(bytes),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
//  Calldata decoders
// ---------------------------------------------------------------------------

pub(crate) fn decode_u256_arg(input: &Bytes) -> Option<U256> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(U256::from_be_slice(&input[4..36]))
}

pub(crate) fn decode_address_arg(input: &Bytes) -> Option<Address> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(Address::from_slice(&input[4 + 12..4 + 32]))
}

pub(crate) fn decode_address_u256_args(input: &Bytes) -> Option<(Address, U256)> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let value = U256::from_be_slice(&input[4 + 32..4 + 64]);
    Some((addr, value))
}

pub(crate) fn decode_address_bytes32_bytes32_args(
    input: &Bytes,
) -> Option<(Address, [u8; 32], [u8; 32])> {
    if input.len() < 4 + 96 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    let mut value = [0u8; 32];
    value.copy_from_slice(&input[4 + 64..4 + 96]);
    Some((addr, slot, value))
}

pub(crate) fn decode_address_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32])> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    Some((addr, slot))
}
