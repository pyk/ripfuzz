//! FFI cheatcode.

use std::process::Command;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, revert_outcome, success_bytes_outcome};

/// `ffi(string[])` returns `bytes`.
pub const FFI_SELECTOR: [u8; 4] = [0x0a, 0x94, 0xd9, 0x2e];

pub fn handle_ffi(
    inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if !inspector.state.ffi_enabled {
        return Some(revert_outcome("ffi disabled: enable via config"));
    }
    let array_type = DynSolType::Array(Box::new(DynSolType::String));
    let decoded = match array_type.abi_decode_params(&input[4..]) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let DynSolValue::Array(args) = decoded else {
        return None;
    };
    if args.is_empty() {
        return Some(revert_outcome("ffi: no command provided"));
    }

    let mut it = args.into_iter();
    let cmd = match it.next()? {
        DynSolValue::String(s) => s,
        _ => return None,
    };
    let mut command = Command::new(&cmd);
    for arg in it {
        if let DynSolValue::String(s) = arg {
            command.arg(&s);
        }
    }

    let output = match command.output() {
        Ok(v) => v,
        Err(_) => return None,
    };
    if !output.status.success() {
        return Some(revert_outcome("ffi command failed"));
    }
    Some(success_bytes_outcome(output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

    #[test]
    fn ffi_disabled_returns_revert() {
        let mut inspector = CheatcodeInspector::new();
        // ffi_enabled defaults to false
        let input = revm::primitives::Bytes::from(vec![
            0x0a, 0x94, 0xd9, 0x2e, // selector
            0x00, 0x00, 0x00, 0x00, // minimal invalid payload
        ]);
        let result = handle_ffi(&mut inspector, &input);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Revert
        );
    }
}
