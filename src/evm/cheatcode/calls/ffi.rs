//! `ffi` cheatcode - execute arbitrary host commands.

use std::process::Command;

use alloy_dyn_abi::DynSolValue;

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle(
    args: Vec<String>,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    if !state.ffi_enabled {
        return Some(outcome::revert(
            "ffi disabled: use --ffi to enable",
            gas_limit,
        ));
    }
    if args.is_empty() {
        return Some(outcome::revert("ffi: empty command", gas_limit));
    }

    let output = match run_ffi(&args, &state.project_root) {
        Ok(out) => out,
        Err(e) => return Some(outcome::revert(&e, gas_limit)),
    };
    let encoded = DynSolValue::Bytes(output).abi_encode();
    Some(outcome::success_bytes(encoded, gas_limit))
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
