//! Shared outcome builders for cheatcode handlers.

use alloy_dyn_abi::DynSolValue;
use revm::{
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, U256},
};

// ---------------------------------------------------------------------------
// Outcome builders
// ---------------------------------------------------------------------------

pub fn success(gas_limit: u64) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Stop,
            output: Bytes::new(),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn success_u256(value: U256, gas_limit: u64) -> CallOutcome {
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

pub fn success_address(value: Address, gas_limit: u64) -> CallOutcome {
    let mut out = vec![0u8; 32];
    out[12..32].copy_from_slice(value.as_slice());
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(out),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn success_bool(value: bool, gas_limit: u64) -> CallOutcome {
    let mut out = vec![0u8; 32];
    if value {
        out[31] = 1;
    }
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(out),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn success_bytes(value: Vec<u8>, gas_limit: u64) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(value),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn success_sign(v: u8, r: [u8; 32], s: [u8; 32], gas_limit: u64) -> CallOutcome {
    let mut out = vec![0u8; 96];
    out[31] = v;
    out[32..64].copy_from_slice(&r);
    out[64..96].copy_from_slice(&s);
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(out),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

pub fn revert(reason: &str, gas_limit: u64) -> CallOutcome {
    let mut encoded = vec![0x08, 0xc3, 0x79, 0xa0];
    encoded.extend_from_slice(&DynSolValue::String(reason.into()).abi_encode());
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from(encoded),
            gas: Gas::new(gas_limit),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}
