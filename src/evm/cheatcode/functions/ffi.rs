//! FFI cheatcode - execute arbitrary host commands.

use std::process::Command;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::evm::cheatcode::{Cheatcode, CheatcodeEffect};

pub struct Ffi;

impl Cheatcode for Ffi {
    type Args = Vec<String>;
    const SELECTOR: [u8; 4] = [0x89, 0x16, 0x04, 0x67];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        let array_type = DynSolType::Array(Box::new(DynSolType::String));
        let decoded = array_type.abi_decode_params(&input[4..]).ok()?;
        let DynSolValue::Array(args) = decoded else {
            return None;
        };
        let mut result = Vec::new();
        for arg in args {
            if let DynSolValue::String(s) = arg {
                result.push(s);
            } else {
                return None;
            }
        }
        Some(result)
    }

    fn effects(args: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::FfiExec(args)]
    }
}

/// Execute an FFI command and encode the result as ABI `bytes`.
///
/// Returns `Err(reason)` if FFI is disabled, the command is empty, or the
/// command exits with a non-zero status code.
pub fn run_ffi(
    args: &[String],
    enabled: bool,
    project_root: &std::path::Path,
) -> Result<Vec<u8>, String> {
    if !enabled {
        return Err(
            "ffi is not enabled in the configuration; add --ffi to allow external commands".into(),
        );
    }
    if args.is_empty() || args[0].is_empty() {
        return Err("ffi: no command was provided".into());
    }

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    if !project_root.as_os_str().is_empty() {
        cmd.current_dir(project_root);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("ffi: failed to execute command: {e}"))?;

    if !output.status.success() {
        let exit = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffi: command exited with code {exit}. stderr: {stderr}"
        ));
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

    Ok(DynSolValue::Bytes(bytes).abi_encode())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn ffi_decode_works() {
        let input = Bytes::from(vec![0x0a, 0x94, 0xd9, 0x2e, 0x00, 0x00, 0x00, 0x00]);
        assert!(Ffi::decode(&input).is_none());
    }

    #[test]
    fn ffi_effects_produces_ffi_exec() {
        let args = vec!["echo".into(), "hello".into()];
        let effects = Ffi::effects(args.clone());
        assert_eq!(effects, vec![CheatcodeEffect::FfiExec(args)]);
    }

    #[test]
    fn run_ffi_disabled() {
        let err = run_ffi(&["echo".into()], false, Path::new(".")).unwrap_err();
        assert!(err.contains("not enabled"));
    }

    #[test]
    fn run_ffi_empty() {
        let err = run_ffi(&[], true, Path::new(".")).unwrap_err();
        assert!(err.contains("no command"));
    }

    #[test]
    fn run_ffi_hex_output() {
        let out = run_ffi(
            &["printf".into(), "%s".into(), "0x6869".into()],
            true,
            Path::new("."),
        )
        .unwrap();
        let decoded = DynSolType::Bytes.abi_decode(&out).unwrap();
        assert_eq!(decoded, DynSolValue::Bytes(vec![0x68, 0x69]));
    }

    #[test]
    fn run_ffi_raw_output() {
        let out = run_ffi(&["echo".into(), "hello".into()], true, Path::new(".")).unwrap();
        let decoded = DynSolType::Bytes.abi_decode(&out).unwrap();
        assert_eq!(decoded, DynSolValue::Bytes("hello\n".as_bytes().to_vec()));
    }

    #[test]
    fn run_ffi_failure_reverts() {
        let err = run_ffi(&["false".into()], true, Path::new(".")).unwrap_err();
        assert!(err.contains("exited with code"));
    }

    // ------------------------------------------------------------------
    // Integration tests
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn cheatcode_ffi_setup_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xc6, 0x4e, 0x6d, 0xa4]; // action_record_hash()
        let calls = vec![Call {
            selector: call_record,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_sequence_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_echo: [u8; 4] = [0x6a, 0xad, 0xa2, 0xed]; // action_ffi_echo(string)
        let mut args = vec![0u8; 32 + 32 + 32];
        // offset to string data
        args[24..32].copy_from_slice(&32u64.to_be_bytes());
        // length = 5
        args[32 + 24..32 + 32].copy_from_slice(&5u64.to_be_bytes());
        // "hello" padded to 32 bytes
        args[32 + 32..32 + 32 + 5].copy_from_slice(b"hello");
        let call_echo = Call {
            selector: action_ffi_echo,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        };
        let call_record: [u8; 4] = [0xc6, 0x4e, 0x6d, 0xa4]; // action_record_hash()
        let call_record_hash = Call {
            selector: call_record,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        };

        let output = chain.execute(&[call_echo, call_record_hash]).unwrap();
        assert!(output.all_ok, "calls should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_revert_integration() {
        let marker = std::path::Path::new("/tmp/raptor_ffi_revert_marker");
        let _ = fs::remove_file(marker);

        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_and_revert: [u8; 4] = [0x38, 0x08, 0x1c, 0x97]; // action_ffi_and_revert()
        let calls = vec![Call {
            selector: action_ffi_and_revert,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "call should revert");
        assert!(
            marker.exists(),
            "host side effect from ffi should survive EVM revert"
        );
        let _ = fs::remove_file(marker);
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_hex_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_hex: [u8; 4] = [0xc8, 0xf1, 0x28, 0x13]; // action_ffi_hex()
        let calls = vec![Call {
            selector: action_ffi_hex,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_raw_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_raw: [u8; 4] = [0x57, 0x30, 0x82, 0x43]; // action_ffi_raw()
        let calls = vec![Call {
            selector: action_ffi_raw,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_empty_revert_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_empty: [u8; 4] = [0x52, 0x9e, 0xac, 0xe7]; // action_ffi_empty()
        let calls = vec![Call {
            selector: action_ffi_empty,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "empty ffi should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_fail_revert_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_fail: [u8; 4] = [0x4d, 0x78, 0xdd, 0xb2]; // action_ffi_fail()
        let calls = vec![Call {
            selector: action_ffi_fail,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "false command should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_corpus_isolation_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();

        // Sequence A: store a new hash via ffi
        let action_ffi_echo: [u8; 4] = [0x6a, 0xad, 0xa2, 0xed]; // action_ffi_echo(string)
        let mut args_a = vec![0u8; 32 + 32 + 32];
        args_a[24..32].copy_from_slice(&32u64.to_be_bytes());
        args_a[32 + 24..32 + 32].copy_from_slice(&1u64.to_be_bytes());
        args_a[32 + 32..32 + 32 + 1].copy_from_slice(b"A");
        let calls_a = vec![Call {
            selector: action_ffi_echo,
            args: args_a,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        // Sequence B: check that hash is back to setup value
        let call_setup_only: [u8; 4] = [0xc6, 0x4e, 0x6d, 0xa4]; // action_record_hash()
        let calls_b = vec![Call {
            selector: call_setup_only,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let output_b = chain.execute(&calls_b).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_invariant_final_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_echo: [u8; 4] = [0x6a, 0xad, 0xa2, 0xed]; // action_ffi_echo(string)
        let mut args = vec![0u8; 32 + 32 + 32];
        args[24..32].copy_from_slice(&32u64.to_be_bytes());
        args[32 + 24..32 + 32].copy_from_slice(&5u64.to_be_bytes());
        args[32 + 32..32 + 32 + 5].copy_from_slice(b"final");
        let calls = vec![Call {
            selector: action_ffi_echo,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_ffi_warp_interaction_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFFI.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .with_project(Path::new("fixtures/cheatcodes"))
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default().with_ffi(true),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let action_ffi_and_warp: [u8; 4] = [0x84, 0x0e, 0xac, 0xeb]; // action_ffi_and_warp()
        let calls = vec![Call {
            selector: action_ffi_and_warp,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
