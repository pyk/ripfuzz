//! Outcome helpers for cheatcode execution.

use alloy_primitives::I256;
use revm::{
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Bytes, U256},
};

use crate::evm::cheatcode::{CheatcodeEffect, ExecutionState};

pub fn build_outcome<CTX: ContextTr>(
    effects: &[CheatcodeEffect],
    gas_limit: u64,
    ctx: &mut CTX,
    state: &ExecutionState,
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
            match crate::evm::cheatcode::functions::ffi::run_ffi(
                args,
                state.ffi_enabled,
                &state.project_root,
            ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_panic_encoding_matches_solidity() {
        let result = panic_outcome();
        let out = result.result.output;
        assert_eq!(&out[..4], &[0x4e, 0x48, 0x7b, 0x71]); // Panic(uint256)
        assert_eq!(&out[4..35], &[0u8; 31]); // padded uint256(1)
        assert_eq!(out[35], 0x01);
    }
}
