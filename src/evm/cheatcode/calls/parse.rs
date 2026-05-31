//! `parse*` cheatcodes - parse strings into Solidity values.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::{Address, U256};

use crate::evm::cheatcode::outcome;

pub fn parse_uint(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let value: U256 = s.parse().ok()?;
    Some(outcome::success_u256(value))
}

pub fn parse_int(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let value: alloy_primitives::I256 = s.parse().ok()?;
    Some(outcome::success_u256(value.into_raw()))
}

pub fn parse_bool(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let trimmed = s.trim();
    let value = match trimmed {
        "1" => true,
        "0" => false,
        t if t.eq_ignore_ascii_case("true") => true,
        t if t.eq_ignore_ascii_case("false") => false,
        _ => return Some(outcome::revert("parseBool: invalid bool string")),
    };
    Some(outcome::success_bool(value))
}

pub fn parse_address(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let value: Address = s.parse().ok()?;
    Some(outcome::success_address(value))
}

pub fn parse_bytes(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let value = hex::decode(stripped).ok()?;
    let encoded = DynSolValue::Bytes(value).abi_encode();
    Some(outcome::success_bytes(encoded))
}

pub fn parse_bytes32(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let value = hex::decode(stripped).ok()?;
    if value.len() != 32 {
        return Some(outcome::revert("parseBytes32: invalid length"));
    }
    Some(outcome::success_bytes(value))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::parse;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface ParseTarget {
            function setup() external;
            function actionReParseAll() external;
            function actionParseSequence() external;
            function actionParseDifferentUint() external;
            function actionRevertInvalidBool() external pure;
            function actionRevertInvalidAddress() external pure;
            function actionRevertInvalidUint() external pure;
            function actionRevertInvalidBytes32() external pure;
            function getStoredUint() external view returns (uint256);
            function invariant_allParsedMatch() external view;
        }
    }

    const EXPECTED_UINT: U256 = U256::from_limbs([123, 0, 0, 0]);
    const EXPECTED_INT: alloy_primitives::I256 =
        alloy_primitives::I256::from_raw(U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xD6,
        ]));
    const EXPECTED_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/ParseTarget.sol:ParseTarget");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // -----------------------------------------------------------------
    // Handler-level unit tests
    // -----------------------------------------------------------------

    /// vm.parseUint("123") must return 123.
    #[test]
    fn parse_uint_handler_level() {
        let outcome = parse::parse_uint("123");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseUint must succeed");
        assert_eq!(
            outcome.result.output,
            Bytes::from(EXPECTED_UINT.to_be_bytes_vec())
        );
    }

    /// vm.parseUint("0x7b") must return 123.
    #[test]
    fn parse_uint_hex_handler_level() {
        let outcome = parse::parse_uint("0x7b");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseUint with hex must succeed");
        assert_eq!(
            outcome.result.output,
            Bytes::from(U256::from(0x7b).to_be_bytes_vec())
        );
    }

    /// vm.parseInt("-42") must return -42.
    #[test]
    fn parse_int_handler_level() {
        let outcome = parse::parse_int("-42");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseInt must succeed");
        assert_eq!(
            outcome.result.output,
            Bytes::from(EXPECTED_INT.into_raw().to_be_bytes_vec())
        );
    }

    /// vm.parseBool("true") must return true.
    #[test]
    fn parse_bool_true_handler_level() {
        let outcome = parse::parse_bool("true");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBool(true) must succeed");
        let mut expected = vec![0u8; 32];
        expected[31] = 1;
        assert_eq!(outcome.result.output, Bytes::from(expected));
    }

    /// vm.parseBool("false") must return false.
    #[test]
    fn parse_bool_false_handler_level() {
        let outcome = parse::parse_bool("false");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBool(false) must succeed");
        assert_eq!(outcome.result.output, Bytes::from(vec![0u8; 32]));
    }

    /// vm.parseBool("1") must return true.
    #[test]
    fn parse_bool_one_handler_level() {
        let outcome = parse::parse_bool("1");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBool(1) must succeed");
        let mut expected = vec![0u8; 32];
        expected[31] = 1;
        assert_eq!(outcome.result.output, Bytes::from(expected));
    }

    /// vm.parseBool("0") must return false.
    #[test]
    fn parse_bool_zero_handler_level() {
        let outcome = parse::parse_bool("0");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBool(0) must succeed");
        assert_eq!(outcome.result.output, Bytes::from(vec![0u8; 32]));
    }

    /// vm.parseBool("maybe") must revert.
    #[test]
    fn parse_bool_invalid_reverts() {
        let outcome = parse::parse_bool("maybe");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.parseBool with invalid string must revert"
        );
    }

    /// vm.parseAddress must return the address.
    #[test]
    fn parse_address_handler_level() {
        let outcome = parse::parse_address("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseAddress must succeed");
        let mut expected = vec![0u8; 32];
        expected[12..32].copy_from_slice(EXPECTED_ADDR.as_slice());
        assert_eq!(outcome.result.output, Bytes::from(expected));
    }

    /// vm.parseBytes32 with valid 64-char hex must return 32 bytes.
    #[test]
    fn parse_bytes32_handler_level() {
        let hex_str = "abababababababababababababababababababababababababababababababab";
        let outcome = parse::parse_bytes32(hex_str);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBytes32 must succeed");
        assert_eq!(outcome.result.output.len(), 32);
    }

    /// vm.parseBytes32 with wrong length must revert.
    #[test]
    fn parse_bytes32_invalid_length_reverts() {
        let outcome = parse::parse_bytes32("abcd");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.parseBytes32 with invalid length must revert"
        );
    }

    /// vm.parseBytes must return ABI-encoded bytes.
    #[test]
    fn parse_bytes_handler_level() {
        let outcome = parse::parse_bytes("0x1234abcd");
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.parseBytes must succeed");
        assert!(
            outcome.result.output.len() > 32,
            "must be ABI-encoded bytes"
        );
    }

    // -----------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------

    /// Values parsed during setup must be readable by an invariant call
    /// executed through `chain.exec`, proving the baseline state is correct.
    #[test]
    fn setup_parsed_values_match_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ParseTarget::invariant_allParsedMatchCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// Re-parsing the same canonical values in an action and then calling the
    /// invariant must leave the state unchanged, proving `vm.parse*` is
    /// deterministic and safe to call repeatedly.
    #[test]
    fn reparse_same_values_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionReParseAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::invariant_allParsedMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionReParseAll must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after re-parsing"
        );
    }

    /// Parsing a different uint value in an action mutates the stored state,
    /// so the invariant must fail afterward. This proves `vm.parseUint` actually
    /// changes contract state rather than being a no-op.
    #[test]
    fn parse_different_value_breaks_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionParseDifferentUintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::invariant_allParsedMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionParseDifferentUint must succeed"
        );
        assert!(
            !execution.results[1].success,
            "invariant must fail after parsing a different value"
        );
    }

    /// A sequence of parse calls inside a single transaction must end on the
    /// correct final value, proving multiple `vm.parseUint` calls in one tx
    /// compose correctly and do not interfere with each other.
    #[test]
    fn parse_sequence_returns_correct_final_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionParseSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::getStoredUintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::invariant_allParsedMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionParseSequence must succeed"
        );
        assert!(execution.results[1].success, "getStoredUint must succeed");
        let stored = ParseTarget::getStoredUintCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(stored, EXPECTED_UINT, "final stored uint must be 123");
        assert!(
            execution.results[2].success,
            "invariant must pass after sequence"
        );
    }

    /// Invalid parse inputs must cause the calling transaction to revert when
    /// executed through `chain.exec`, proving error propagation works for all
    /// parse types in the real fuzzing path.
    #[test]
    fn invalid_parse_inputs_revert_in_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionRevertInvalidBoolCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionRevertInvalidAddressCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionRevertInvalidUintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ParseTarget::actionRevertInvalidBytes32Call::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(!execution.results[0].success, "invalid bool must revert");
        assert!(!execution.results[1].success, "invalid address must revert");
        assert!(!execution.results[2].success, "invalid uint must revert");
        assert!(!execution.results[3].success, "invalid bytes32 must revert");
    }

    /// A cloned chain snapshot must produce the same parse state when the
    /// invariant is executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_parse_state() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ParseTarget::invariant_allParsedMatchCall::new(()).abi_encode(),
        ))];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass on cloned chain"
        );
    }
}
