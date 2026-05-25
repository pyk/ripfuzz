//! Shared decode helpers and outcome builders for cheatcode handlers.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::{
    interpreter::{CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, U256},
};

// ---------------------------------------------------------------------------
// Decode helpers (use DynAbi for strongly typed decoding)
// ---------------------------------------------------------------------------

pub fn decode_params(input: &Bytes, types: &[DynSolType]) -> Option<Vec<DynSolValue>> {
    let ty = DynSolType::Tuple(types.to_vec());
    let DynSolValue::Tuple(vals) = ty.abi_decode_params(&input[4..]).ok()? else {
        return None;
    };
    Some(vals)
}

pub fn decode_u256(input: &Bytes) -> Option<U256> {
    let mut vals = decode_params(input, &[DynSolType::Uint(256)])?;
    let DynSolValue::Uint(v, _) = vals.pop()? else {
        return None;
    };
    Some(v)
}

pub fn decode_address(input: &Bytes) -> Option<Address> {
    let mut vals = decode_params(input, &[DynSolType::Address])?;
    let DynSolValue::Address(a) = vals.pop()? else {
        return None;
    };
    Some(a)
}

pub fn decode_address_u256(input: &Bytes) -> Option<(Address, U256)> {
    let mut vals = decode_params(input, &[DynSolType::Address, DynSolType::Uint(256)])?;
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::Uint(v, _) = vals.remove(0) else {
        return None;
    };
    Some((a, v))
}

pub fn decode_address_bytes32(input: &Bytes) -> Option<(Address, [u8; 32])> {
    let mut vals = decode_params(input, &[DynSolType::Address, DynSolType::FixedBytes(32)])?;
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::FixedBytes(b, _) = vals.remove(0) else {
        return None;
    };
    Some((a, b.into()))
}

pub fn decode_address_bytes32_bytes32(input: &Bytes) -> Option<(Address, [u8; 32], [u8; 32])> {
    let mut vals = decode_params(
        input,
        &[
            DynSolType::Address,
            DynSolType::FixedBytes(32),
            DynSolType::FixedBytes(32),
        ],
    )?;
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::FixedBytes(b1, _) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::FixedBytes(b2, _) = vals.remove(0) else {
        return None;
    };
    Some((a, b1.into(), b2.into()))
}

pub fn decode_address_bytes(input: &Bytes) -> Option<(Address, Bytes)> {
    let mut vals = decode_params(input, &[DynSolType::Address, DynSolType::Bytes])?;
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::Bytes(b) = vals.remove(0) else {
        return None;
    };
    Some((a, Bytes::from(b)))
}

pub fn decode_address_string(input: &Bytes) -> Option<(Address, String)> {
    let mut vals = decode_params(input, &[DynSolType::Address, DynSolType::String])?;
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::String(s) = vals.remove(0) else {
        return None;
    };
    Some((a, s))
}

pub fn decode_string(input: &Bytes) -> Option<String> {
    let mut vals = decode_params(input, &[DynSolType::String])?;
    let DynSolValue::String(s) = vals.pop()? else {
        return None;
    };
    Some(s)
}

pub fn decode_u256_bytes32(input: &Bytes) -> Option<(U256, [u8; 32])> {
    let mut vals = decode_params(input, &[DynSolType::Uint(256), DynSolType::FixedBytes(32)])?;
    let DynSolValue::Uint(v, _) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::FixedBytes(b, _) = vals.remove(0) else {
        return None;
    };
    Some((v, b.into()))
}

pub fn decode_u256_address(input: &Bytes) -> Option<(U256, Address)> {
    let mut vals = decode_params(input, &[DynSolType::Uint(256), DynSolType::Address])?;
    let DynSolValue::Uint(v, _) = vals.remove(0) else {
        return None;
    };
    let DynSolValue::Address(a) = vals.remove(0) else {
        return None;
    };
    Some((v, a))
}

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
