//! Foundry-compatible VM contract: cheatcodes, state, and dispatch.
//!
//! The public surface is intentionally small: [`VM_ADDRESS`] and [`VmConfig`].
//! Everything else is `pub` so that `chain/` can access it, but the module
//! structure makes the boundary clear.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alloy_primitives::I256;
use revm::{
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, U256},
};

pub use cheatcodes::deal::DealRecord;
pub use cheatcodes::nonce::NonceRecord;

use crate::vm::effect::CheatcodeEffect;

pub mod cheatcodes;
pub mod effect;
pub mod inspector;

/// Raptor VM contract address.
///
/// Derived from `address(uint160(uint256(keccak256("raptor vm"))))`.
///
/// NOTE: The raptor VM is **not** Foundry VM compatible.  It does not
/// implement all Foundry cheatcodes — only the subset documented in the
/// raptor cheatcode module.
pub const VM_ADDRESS: Address = Address::new([
    0x26, 0x3a, 0xf5, 0x13, 0xa0, 0x43, 0x5e, 0xbc, 0x9d, 0x5c, 0x36, 0x2c, 0xf7, 0x62, 0x52, 0xf8,
    0x71, 0x73, 0xf8, 0xf1,
]);

/// User-facing configuration for the VM contract.
#[derive(Debug, Clone)]
pub struct VmConfig {
    /// Enable `vm.ffi` (allows arbitrary host command execution).
    pub ffi: bool,
    /// Foundry project root used by `vm.getCode` and `vm.ffi`.
    pub project_root: PathBuf,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            ffi: false,
            project_root: PathBuf::new(),
        }
    }
}

impl VmConfig {
    pub fn with_ffi(mut self, enabled: bool) -> Self {
        self.ffi = enabled;
        self
    }

    pub fn with_project_root(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }
}

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

pub fn dispatch_effects(sel: [u8; 4], input: &Bytes) -> Option<Vec<CheatcodeEffect>> {
    match sel {
        // Block / state manipulation
        cheatcodes::warp::Warp::SELECTOR => dispatch::<cheatcodes::warp::Warp>(input),
        cheatcodes::roll::Roll::SELECTOR => dispatch::<cheatcodes::roll::Roll>(input),
        cheatcodes::fee::Fee::SELECTOR => dispatch::<cheatcodes::fee::Fee>(input),
        cheatcodes::coinbase::Coinbase::SELECTOR => {
            dispatch::<cheatcodes::coinbase::Coinbase>(input)
        }
        cheatcodes::prevrandao::Prevrandao::SELECTOR => {
            dispatch::<cheatcodes::prevrandao::Prevrandao>(input)
        }
        cheatcodes::chain_id::ChainId::SELECTOR => dispatch::<cheatcodes::chain_id::ChainId>(input),
        cheatcodes::difficulty::Difficulty::SELECTOR => {
            dispatch::<cheatcodes::difficulty::Difficulty>(input)
        }

        // Account manipulation
        cheatcodes::deal::Deal::SELECTOR => dispatch::<cheatcodes::deal::Deal>(input),
        cheatcodes::etch::Etch::SELECTOR => dispatch::<cheatcodes::etch::Etch>(input),
        cheatcodes::nonce::SetNonce::SELECTOR => dispatch::<cheatcodes::nonce::SetNonce>(input),
        cheatcodes::nonce::GetNonce::SELECTOR => dispatch::<cheatcodes::nonce::GetNonce>(input),
        cheatcodes::storage::Load::SELECTOR => dispatch::<cheatcodes::storage::Load>(input),
        cheatcodes::storage::Store::SELECTOR => dispatch::<cheatcodes::storage::Store>(input),

        // Prank
        cheatcodes::prank::Prank::SELECTOR => dispatch::<cheatcodes::prank::Prank>(input),
        cheatcodes::prank::PrankOrigin::SELECTOR => {
            dispatch::<cheatcodes::prank::PrankOrigin>(input)
        }
        cheatcodes::prank::StartPrank::SELECTOR => dispatch::<cheatcodes::prank::StartPrank>(input),
        cheatcodes::prank::StartPrankOrigin::SELECTOR => {
            dispatch::<cheatcodes::prank::StartPrankOrigin>(input)
        }
        cheatcodes::prank::StopPrank::SELECTOR => dispatch::<cheatcodes::prank::StopPrank>(input),

        // Label
        cheatcodes::label::Label::SELECTOR => dispatch::<cheatcodes::label::Label>(input),
        cheatcodes::label::GetLabel::SELECTOR => dispatch::<cheatcodes::label::GetLabel>(input),

        // String / type conversion
        cheatcodes::to_string::ToStringAddress::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringAddress>(input)
        }
        cheatcodes::to_string::ToStringBool::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringBool>(input)
        }
        cheatcodes::to_string::ToStringUint::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringUint>(input)
        }
        cheatcodes::to_string::ToStringInt::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringInt>(input)
        }
        cheatcodes::to_string::ToStringBytes32::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringBytes32>(input)
        }
        cheatcodes::to_string::ToStringBytes::SELECTOR => {
            dispatch::<cheatcodes::to_string::ToStringBytes>(input)
        }
        cheatcodes::parse::ParseUint::SELECTOR => dispatch::<cheatcodes::parse::ParseUint>(input),
        cheatcodes::parse::ParseInt::SELECTOR => dispatch::<cheatcodes::parse::ParseInt>(input),
        cheatcodes::parse::ParseBool::SELECTOR => dispatch::<cheatcodes::parse::ParseBool>(input),
        cheatcodes::parse::ParseAddress::SELECTOR => {
            dispatch::<cheatcodes::parse::ParseAddress>(input)
        }
        cheatcodes::parse::ParseBytes::SELECTOR => dispatch::<cheatcodes::parse::ParseBytes>(input),
        cheatcodes::parse::ParseBytes32::SELECTOR => {
            dispatch::<cheatcodes::parse::ParseBytes32>(input)
        }
        cheatcodes::get_code::GetCode::SELECTOR => dispatch::<cheatcodes::get_code::GetCode>(input),

        // Wallet / crypto
        cheatcodes::addr::Addr::SELECTOR => dispatch::<cheatcodes::addr::Addr>(input),
        cheatcodes::sign::Sign::SELECTOR => dispatch::<cheatcodes::sign::Sign>(input),

        // FFI
        cheatcodes::ffi::Ffi::SELECTOR => dispatch::<cheatcodes::ffi::Ffi>(input),

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
    /// The original `tx.origin` before any prank was applied.
    /// Stored in `VmState` so it survives EVM rebuilds.
    pub original_origin: Option<Address>,
}

impl PrankCheatState {
    /// Return the caller that should be used for the top-level transaction.
    pub fn caller_for_top_level(&self) -> Option<Address> {
        self.start.as_ref().map(|s| s.caller)
    }

    /// Return the origin that should be used for the top-level transaction.
    pub fn origin_for_top_level(&self, default: Address) -> Address {
        self.start
            .as_ref()
            .and_then(|s| s.origin)
            .unwrap_or(default)
    }
}

/// State accumulated by cheatcodes during execution.
#[derive(Clone, Debug, Default)]
pub struct VmState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub ffi_enabled: bool,
    /// Foundry project root used as the working directory for `vm.ffi`.
    pub project_root: PathBuf,
    /// Contract name -> initcode bytes, populated from the artifact so
    /// `vm.getCode` can resolve contracts by name.
    pub compiled_contracts: HashMap<String, Bytes>,
    /// Rollback records for `vm.deal` (Foundry semantics).
    pub eth_deals: Vec<DealRecord>,
    /// Rollback records for `vm.setNonce`.
    pub nonce_changes: Vec<NonceRecord>,
}

impl VmState {
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

/// Block-context overrides produced from `VmState`.
#[derive(Clone, Debug, Default)]
pub struct BlockOverrides {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<u64>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<revm::primitives::FixedBytes<32>>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    pub single_call: bool,
    /// Call depth of the frame that configured this prank.
    pub set_depth: u64,
    /// Address of the contract that called the cheatcode (prank initiator).
    pub prank_caller: Address,
    /// Whether the prank has been applied at least once.
    pub used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartPrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    /// Call depth at which this prank was set.
    pub set_depth: u64,
    /// Address of the contract that called the cheatcode (prank initiator).
    pub prank_caller: Address,
    /// Whether the prank has been applied at least once.
    pub used: bool,
}

// ---------------------------------------------------------------------------
//  Outcome helpers
// ---------------------------------------------------------------------------

pub fn build_outcome<CTX: ContextTr>(
    effects: &[CheatcodeEffect],
    gas_limit: u64,
    ctx: &mut CTX,
    state: &VmState,
) -> CallOutcome {
    if let Some(outcome) = effects.iter().find_map(|effect| match effect {
        CheatcodeEffect::Revert(reason) => Some(revert_outcome(reason)),
        CheatcodeEffect::Panic => Some(panic_outcome()),
        CheatcodeEffect::ReturnU256(v) => Some(success_u256_outcome(*v, gas_limit)),
        CheatcodeEffect::ReturnInt256(v) => Some(success_int256_outcome(*v, gas_limit)),
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
            // Reject precompiles (Foundry-compatible).
            if ctx.journal().precompile_addresses().contains(addr) {
                return Some(revert_outcome("load: cannot read from precompile"));
            }
            // Intent is read-only, but revm's `sload` lives on `JournaledAccountTr`
            // which requires `&mut self` to update cold/warm tracking.  We keep
            // `load_account_mut` for API compatibility but do not mutate storage.
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
        CheatcodeEffect::GetLabel(addr) => {
            let name = state.labels.get(addr).cloned().unwrap_or_default();
            Some(success_bytes_outcome(
                alloy_dyn_abi::DynSolValue::String(name).abi_encode(),
                gas_limit,
            ))
        }
        CheatcodeEffect::GetCode(name) => {
            let Some(initcode) = state.compiled_contracts.get(name) else {
                return Some(revert_outcome(&format!(
                    "getCode: contract not found: {name}"
                )));
            };
            if initcode.is_empty() {
                return Some(revert_outcome(&format!(
                    "getCode: contract bytecode is empty: {name}"
                )));
            }
            Some(success_bytes_outcome(
                alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode(),
                gas_limit,
            ))
        }
        CheatcodeEffect::FfiExec(args) => {
            match crate::vm::cheatcodes::ffi::run_ffi(args, state.ffi_enabled, &state.project_root)
            {
                Ok(encoded) => Some(success_bytes_outcome(encoded, gas_limit)),
                Err(reason) => Some(revert_outcome(&reason)),
            }
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

pub fn dummy_success() -> CallOutcome {
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

pub fn panic_outcome() -> CallOutcome {
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

pub fn revert_outcome(reason: &str) -> CallOutcome {
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

pub fn success_u256_outcome(value: U256, gas_limit: u64) -> CallOutcome {
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

pub fn success_int256_outcome(value: I256, gas_limit: u64) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(value.into_raw().to_be_bytes_vec()),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn success_bool_outcome(value: bool, gas_limit: u64) -> CallOutcome {
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

pub fn success_bytes_outcome(bytes: Vec<u8>, gas_limit: u64) -> CallOutcome {
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

pub fn decode_u256_arg(input: &Bytes) -> Option<U256> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(U256::from_be_slice(&input[4..36]))
}

pub fn decode_address_arg(input: &Bytes) -> Option<Address> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(Address::from_slice(&input[4 + 12..4 + 32]))
}

pub fn decode_address_u256_args(input: &Bytes) -> Option<(Address, U256)> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let value = U256::from_be_slice(&input[4 + 32..4 + 64]);
    Some((addr, value))
}

pub fn decode_address_bytes32_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32], [u8; 32])> {
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

pub fn decode_address_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32])> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    Some((addr, slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::keccak256;

    #[test]
    fn assert_panic_encoding_matches_solidity() {
        let result = panic_outcome();
        let out = result.result.output;
        assert_eq!(&out[..4], &[0x4e, 0x48, 0x7b, 0x71]); // Panic(uint256)
        assert_eq!(&out[4..35], &[0u8; 31]); // padded uint256(1)
        assert_eq!(out[35], 0x01);
    }

    #[test]
    fn vm_address_matches_raptor_vm_string() {
        let hash = keccak256(b"raptor vm");
        let expected = Address::from_word(hash);
        assert_eq!(expected, VM_ADDRESS);
    }
}
