//! `ffi` cheatcode - execute arbitrary host commands.

use std::process::Command;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x89, 0x16, 0x04, 0x67];

pub fn handle(
    input: &Bytes,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    if !state.ffi_enabled {
        return Some(util::revert("ffi disabled: use --ffi to enable", gas_limit));
    }

    let ty = DynSolType::Tuple(vec![DynSolType::Array(Box::new(DynSolType::String))]);
    let DynSolValue::Tuple(mut vals) = ty.abi_decode_params(&input[4..]).ok()? else {
        return Some(util::revert("ffi: failed to decode args", gas_limit));
    };
    let DynSolValue::Array(arr) = vals.pop()? else {
        return Some(util::revert("ffi: expected string[]", gas_limit));
    };
    let mut args = Vec::with_capacity(arr.len());
    for v in arr {
        let DynSolValue::String(s) = v else {
            return Some(util::revert("ffi: expected string[]", gas_limit));
        };
        args.push(s);
    }
    if args.is_empty() {
        return Some(util::revert("ffi: empty command", gas_limit));
    }

    let output = match run_ffi(&args, &state.project_root) {
        Ok(out) => out,
        Err(e) => return Some(util::revert(&e, gas_limit)),
    };
    let encoded = DynSolValue::Bytes(output).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}

fn run_ffi(args: &[String], project_root: &std::path::Path) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(&args[0]);
    cmd.current_dir(project_root);
    cmd.args(&args[1..]);
    let out = cmd.output().map_err(|e| format!("ffi failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("ffi command failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let hex = stdout.trim();
    let bytes = hex::decode(hex.strip_prefix("0x").unwrap_or(hex))
        .map_err(|e| format!("ffi output is not valid hex: {e}"))?;
    Ok(bytes)
}
