//! `getCode` cheatcode.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::vm::{Cheatcode, CheatcodeEffect};

fn decode_single(input: &Bytes, t: DynSolType) -> Option<DynSolValue> {
    let tuple = DynSolType::Tuple(vec![t]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) => v.into_iter().next(),
        _ => None,
    }
}

/// Parse a contract path argument into a contract name.
///
/// Supported formats (matching Medusa/Foundry basics):
/// - `"ContractName"`
/// - `"ContractName.sol"`
/// - `"ContractName.sol:ContractName"`
/// - `"path/to/ContractName.sol:ContractName"`
/// - `"ContractName:ContractName"` (name with colon)
fn parse_contract_path(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // Strip optional version suffix `:0.8.23` from the end.
    // We do this by splitting on `:` and checking if the last segment looks
    // like a version (contains a dot). If so, drop it.
    let mut parts: Vec<&str> = input.split(':').collect();
    if let Some(last) = parts.last()
        && last.contains('.')
        && last.chars().any(|c| c.is_ascii_digit())
    {
        parts.pop();
    }

    let without_version = parts.join(":");

    // If there is a colon left, the last segment is the contract name.
    if let Some(pos) = without_version.rfind(':') {
        let name = without_version[pos + 1..].trim();
        if !name.is_empty() {
            return Some(name.into());
        }
    }

    // No colon — treat the whole thing as the contract name, stripping `.sol`
    // if present.
    let name = without_version.trim();
    let name = name.strip_suffix(".sol").unwrap_or(name).trim();
    if !name.is_empty() {
        return Some(name.into());
    }

    None
}

pub struct GetCode;

impl Cheatcode for GetCode {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x8d, 0x1c, 0xc9, 0x25];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }

    fn effects(arg: Self::Args) -> Vec<CheatcodeEffect> {
        let name = parse_contract_path(&arg).unwrap_or(arg);
        vec![CheatcodeEffect::GetCode(name)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serial_test::serial;

    use revm::{MainContext, context::Context, database::InMemoryDB};

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;
    use crate::vm::build_outcome;
    use crate::vm::inspector::CheatcodeInspector;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn get_code_decode_and_effects() {
        let encoded = DynSolValue::String("Helper".into()).abi_encode();
        let args = GetCode::decode(&call_data(GetCode::SELECTOR, encoded)).unwrap();
        let effects = GetCode::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::GetCode("Helper".into())]);
    }

    #[test]
    fn get_code_reverts_on_missing_contract() {
        let state = CheatcodeInspector::new().state;
        let effects = vec![CheatcodeEffect::GetCode("Missing".into())];
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let outcome = build_outcome(&effects, 30_000_000, &mut ctx, &state);
        assert_eq!(
            outcome.result.result,
            revm::interpreter::InstructionResult::Revert
        );
        let reason = String::from_utf8_lossy(&outcome.result.output);
        assert!(
            reason.contains("getCode: contract not found: Missing"),
            "{reason}"
        );
    }

    #[test]
    fn get_code_reverts_on_empty_bytecode() {
        let mut inspector = CheatcodeInspector::new();
        inspector
            .state
            .compiled_contracts
            .insert("Empty".into(), Bytes::new());
        let effects = vec![CheatcodeEffect::GetCode("Empty".into())];
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let outcome = build_outcome(&effects, 30_000_000, &mut ctx, &inspector.state);
        assert_eq!(
            outcome.result.result,
            revm::interpreter::InstructionResult::Revert
        );
        let reason = String::from_utf8_lossy(&outcome.result.output);
        assert!(
            reason.contains("getCode: contract bytecode is empty: Empty"),
            "{reason}"
        );
    }

    #[test]
    fn parse_contract_path_bare_name() {
        assert_eq!(parse_contract_path("Helper"), Some("Helper".into()));
    }

    #[test]
    fn parse_contract_path_sol_suffix() {
        assert_eq!(parse_contract_path("Helper.sol"), Some("Helper".into()));
    }

    #[test]
    fn parse_contract_path_file_colon_name() {
        assert_eq!(
            parse_contract_path("Helper.sol:Helper"),
            Some("Helper".into())
        );
    }

    #[test]
    fn parse_contract_path_with_version() {
        assert_eq!(
            parse_contract_path("Helper.sol:Helper:0.8.23"),
            Some("Helper".into())
        );
    }

    #[test]
    fn parse_contract_path_name_colon_version() {
        assert_eq!(parse_contract_path("Helper:0.8.23"), Some("Helper".into()));
    }

    #[test]
    fn parse_contract_path_path_prefix() {
        assert_eq!(
            parse_contract_path("src/Helper.sol:Helper"),
            Some("Helper".into())
        );
    }

    #[test]
    fn parse_contract_path_empty_returns_none() {
        assert_eq!(parse_contract_path(""), None);
    }

    #[test]
    fn parse_contract_path_whitespace_returns_none() {
        assert_eq!(parse_contract_path("   "), None);
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn cheatcode_get_code_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "setup should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_get_code_action_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let action_deploy: [u8; 4] = [0xa7, 0x6f, 0x21, 0x8b]; // action_deployHelper()

        let calls = vec![Call {
            selector: action_deploy,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_get_code_multi_call_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let action_deploy1: [u8; 4] = [0xa7, 0x6f, 0x21, 0x8b]; // action_deployHelper()
        let action_deploy2: [u8; 4] = [0x74, 0xad, 0xfb, 0xd7]; // action_deployHelperAgain()

        let calls = vec![
            Call {
                selector: action_deploy1,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_deploy2,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "multi-call actions should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_get_code_error_case_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let action_missing: [u8; 4] = [0x2a, 0xb2, 0xe8, 0x5b]; // action_getMissingCode()

        let calls = vec![Call {
            selector: action_missing,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(
            output.all_ok,
            "action that catches the revert should still succeed"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_get_code_formats_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let action_bare: [u8; 4] = [0x15, 0x2a, 0x1e, 0xa2]; // action_getCodeBare()
        let action_file: [u8; 4] = [0x67, 0x16, 0x21, 0xfb]; // action_getCodeFile()
        let action_full: [u8; 4] = [0x53, 0xad, 0x30, 0x1e]; // action_getCodeFull()

        let calls = vec![
            Call {
                selector: action_bare,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_file,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_full,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "format actions should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_get_code_self_lookup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeGetCode.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let action_self: [u8; 4] = [0x8b, 0xce, 0x43, 0x9b]; // action_getSelfCode()

        let calls = vec![Call {
            selector: action_self,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "self-lookup action should succeed");
    }
}
