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

    use crate::evm::chain::{Chain, Config, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::parse;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface ParseTarget {
            function getParsedUint() external view returns (uint256);
            function getParsedInt() external view returns (int256);
            function getParsedBool() external view returns (bool);
            function getParsedAddress() external view returns (address);
            function getParsedBytes() external view returns (bytes memory);
            function getParsedBytes32() external view returns (bytes32);
            function callParseUintSameValueTwice() external pure returns (uint256 first, uint256 second);
            function callParseBytes32SameValueTwice() external pure returns (bytes32 first, bytes32 second);
            function callParseUintSequence() external pure returns (uint256 first, uint256 second, uint256 third);
            function callParseBoolSequence() external pure returns (bool first, bool second, bool third);
            function parseInvalidBool() external pure;
            function parseInvalidAddress() external pure;
            function parseInvalidBytes32Length() external pure;
            function parseInvalidUint() external pure;
            function callParseAndDeal() external returns (uint256 parsed, uint256 balance);
            function callParseAndWarp() external returns (uint256 parsed, uint256 timestamp);
            function callParseAndChainId() external returns (uint256 parsed, uint256 chainId);
            function setup() external;
            function actionParseUint() external;
            function actionParseBytes32() external;
            function actionParseAndDeal() external;
            function getBalance() external view returns (uint256);
            function invariant_parsed_uint() external view;
            function invariant_parsed_int() external view;
            function invariant_parsed_bool() external view;
            function invariant_parsed_address() external view;
            function invariant_parsed_bytes32() external view;
        }
    }

    const EXPECTED_UINT: U256 = U256::from_limbs([123, 0, 0, 0]);
    const EXPECTED_INT: alloy_primitives::I256 =
        alloy_primitives::I256::from_raw(U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xD6,
        ])); // -42 as I256
    const EXPECTED_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const EXPECTED_BYTES32: [u8; 32] = [
        0x74, 0x65, 0x73, 0x74, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/ParseTarget.sol:ParseTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_opts = SetupInput::new(target);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with the cheatcode inspector enabled so that `vm.*`
    /// functions invoked by the target contract are intercepted.
    fn call_with_cheatcode_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = cheatcode::Inspector::default();
        let tx = revm::context::TxEnv {
            caller,
            kind: revm::primitives::TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        let (result, _) = chain.inspect(tx, inspector).unwrap();
        result
    }

    /// Call a view/pure function that returns a single `uint256` and decode it.
    macro_rules! call_uint256_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// Call a view/pure function that returns a single `int256` and decode it.
    macro_rules! call_int256_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// Call a view/pure function that returns a single `bool` and decode it.
    macro_rules! call_bool_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// Call a view/pure function that returns a single `address` and decode it.
    macro_rules! call_address_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// Call a view/pure function that returns `bytes32` and decode it.
    macro_rules! call_bytes32_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// Call a view/pure function that returns `bytes memory` and decode it.
    macro_rules! call_bytes_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    // -----------------------------------------------------------------
    // Handler-level (direct Rust unit tests)
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
        // ABI encoding for bytes: offset (32) | length (4) | data (padded)
        assert!(
            outcome.result.output.len() > 32,
            "must be ABI-encoded bytes"
        );
    }

    // -----------------------------------------------------------------
    // Basic contract-path integration
    // -----------------------------------------------------------------

    /// Values parsed during setup must match expectations.
    #[test]
    fn parse_setup_values_persist_in_storage() {
        let (mut chain, target) = deploy_and_setup();

        let uint_val = call_uint256_getter!(&mut chain, target, ParseTarget::getParsedUintCall);
        assert_eq!(uint_val, EXPECTED_UINT, "stored uint must match");

        let int_val = call_int256_getter!(&mut chain, target, ParseTarget::getParsedIntCall);
        assert_eq!(int_val, EXPECTED_INT, "stored int must match");

        let bool_val = call_bool_getter!(&mut chain, target, ParseTarget::getParsedBoolCall);
        assert!(bool_val, "stored bool must be true");

        let addr_val = call_address_getter!(&mut chain, target, ParseTarget::getParsedAddressCall);
        assert_eq!(addr_val, EXPECTED_ADDR, "stored address must match");

        let b32_val = call_bytes32_getter!(&mut chain, target, ParseTarget::getParsedBytes32Call);
        assert_eq!(
            b32_val.as_slice(),
            EXPECTED_BYTES32,
            "stored bytes32 must match"
        );

        let bytes_val = call_bytes_getter!(&mut chain, target, ParseTarget::getParsedBytesCall);
        assert_eq!(
            bytes_val.as_ref(),
            &[0x12, 0x34, 0xab, 0xcd],
            "stored bytes must match"
        );
    }

    // -----------------------------------------------------------------
    // Single-transaction determinism / sequence
    // -----------------------------------------------------------------

    /// vm.parseUint with the same string twice in one tx must yield identical
    /// results, proving the cheatcode is deterministic and stateless.
    #[test]
    fn parse_uint_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseUintSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseUintSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            ParseTarget::callParseUintSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same parse string must give identical values"
        );
        assert_eq!(ret.first, U256::from(456));
    }

    /// vm.parseBytes32 with the same string twice in one tx must yield identical
    /// results.
    #[test]
    fn parse_bytes32_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseBytes32SameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callParseBytes32SameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret =
            ParseTarget::callParseBytes32SameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same parse string must give identical bytes32"
        );
    }

    /// vm.parseUint with different strings interleaved must produce distinct
    /// values, and repeating a string must reproduce the original value.
    #[test]
    fn parse_uint_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseUintSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseUintSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = ParseTarget::callParseUintSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first,
            U256::from(1),
            "first vm.parseUint(1) must read 1"
        );
        assert_eq!(
            ret.second,
            U256::from(2),
            "second vm.parseUint(2) must read 2"
        );
        assert_eq!(
            ret.third,
            U256::from(1),
            "third vm.parseUint(1) must read 1 again"
        );
        assert_eq!(ret.first, ret.third, "repeated parse must match");
    }

    /// vm.parseBool with different strings interleaved must produce distinct
    /// bools and repeating a string must reproduce the original bool.
    #[test]
    fn parse_bool_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseBoolSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseBoolSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = ParseTarget::callParseBoolSequenceCall::abi_decode_returns(&output).unwrap();
        assert!(ret.first, "first vm.parseBool(true) must be true");
        assert!(!ret.second, "second vm.parseBool(false) must be false");
        assert!(ret.third, "third vm.parseBool(true) must be true again");
        assert_eq!(ret.first, ret.third, "repeated parse must match");
    }

    // -----------------------------------------------------------------
    // Edge cases - revert via contract
    // -----------------------------------------------------------------

    /// vm.parseBool("maybe") must revert when called through the target.
    #[test]
    fn parse_invalid_bool_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::parseInvalidBoolCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "parseInvalidBool() must revert when vm.parseBool is called with invalid string"
        );
    }

    /// vm.parseAddress("not_an_address") must revert.
    #[test]
    fn parse_invalid_address_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::parseInvalidAddressCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "parseInvalidAddress() must revert when vm.parseAddress is called with invalid string"
        );
    }

    /// vm.parseBytes32("abcd") must revert because length is wrong.
    #[test]
    fn parse_invalid_bytes32_length_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::parseInvalidBytes32LengthCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "parseInvalidBytes32Length() must revert when vm.parseBytes32 has wrong length"
        );
    }

    /// vm.parseUint("not_a_number") must revert.
    #[test]
    fn parse_invalid_uint_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::parseInvalidUintCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "parseInvalidUint() must revert when vm.parseUint is called with invalid string"
        );
    }

    // -----------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------

    /// Values parsed and stored during setup must still be readable in later
    /// transactions, proving contract-level persistence works.
    #[test]
    fn parse_setup_values_readable_across_transactions() {
        let (mut chain, target) = deploy_and_setup();

        let first = call_uint256_getter!(&mut chain, target, ParseTarget::getParsedUintCall);
        let second = call_uint256_getter!(&mut chain, target, ParseTarget::getParsedUintCall);
        assert_eq!(first, EXPECTED_UINT);
        assert_eq!(second, EXPECTED_UINT);
        assert_eq!(
            first, second,
            "getter must return the same stored uint across calls"
        );

        let first_b32 = call_bytes32_getter!(&mut chain, target, ParseTarget::getParsedBytes32Call);
        let second_b32 =
            call_bytes32_getter!(&mut chain, target, ParseTarget::getParsedBytes32Call);
        assert_eq!(first_b32.as_slice(), EXPECTED_BYTES32);
        assert_eq!(second_b32.as_slice(), EXPECTED_BYTES32);
    }

    // -----------------------------------------------------------------
    // Fuzzing-scenario coverage (actions + invariants across transactions)
    // -----------------------------------------------------------------

    /// Invariants must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let invariants = [
            (
                ParseTarget::invariant_parsed_uintCall::new(()).abi_encode(),
                "invariant_parsed_uint",
            ),
            (
                ParseTarget::invariant_parsed_intCall::new(()).abi_encode(),
                "invariant_parsed_int",
            ),
            (
                ParseTarget::invariant_parsed_boolCall::new(()).abi_encode(),
                "invariant_parsed_bool",
            ),
            (
                ParseTarget::invariant_parsed_addressCall::new(()).abi_encode(),
                "invariant_parsed_address",
            ),
            (
                ParseTarget::invariant_parsed_bytes32Call::new(()).abi_encode(),
                "invariant_parsed_bytes32",
            ),
        ];
        for (calldata, name) in invariants {
            let result = chain
                .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{name} must pass after setup");
        }
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves `vm.parse*` stays deterministic across multiple transactions
    /// and that invariants correctly observe the persisted state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Action 1: re-parse the expected uint and store it.
        let calldata = ParseTarget::actionParseUintCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionParseUint must succeed");

        // Invariant must still pass after the action.
        let calldata = ParseTarget::invariant_parsed_uintCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_parsed_uint must pass after actionParseUint"
        );

        // Action 2: re-parse the expected bytes32 and store it.
        let calldata = ParseTarget::actionParseBytes32Call::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionParseBytes32 must succeed");

        // Invariant must still pass.
        let calldata = ParseTarget::invariant_parsed_bytes32Call::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_parsed_bytes32 must pass after actionParseBytes32"
        );

        // Action 3: parse a value and use it with deal.
        let calldata = ParseTarget::actionParseAndDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionParseAndDeal must succeed");

        let balance: U256 = call_uint256_getter!(&mut chain, target, ParseTarget::getBalanceCall);
        assert_eq!(
            balance,
            U256::from(1000),
            "balance must be 1000 after actionParseAndDeal"
        );
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    /// vm.parseUint followed by vm.deal must set the correct balance.
    #[test]
    fn parse_and_deal_interaction() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseAndDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseAndDeal must succeed");
        let output = result.output.expect("must return output");
        let ret = ParseTarget::callParseAndDealCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.parsed, U256::from(1000), "parsed value must be 1000");
        assert_eq!(
            ret.balance,
            U256::from(1000),
            "balance must match parsed value"
        );
    }

    /// vm.parseUint followed by vm.warp must set the correct timestamp.
    #[test]
    fn parse_and_warp_interaction() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseAndWarp must succeed");
        let output = result.output.expect("must return output");
        let ret = ParseTarget::callParseAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.parsed,
            U256::from(1234567890),
            "parsed value must be 1234567890"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1234567890),
            "timestamp must match parsed value"
        );
    }

    /// vm.parseUint followed by vm.chainId must set the correct chain id.
    #[test]
    fn parse_and_chain_id_interaction() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ParseTarget::callParseAndChainIdCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callParseAndChainId must succeed");
        let output = result.output.expect("must return output");
        let ret = ParseTarget::callParseAndChainIdCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.parsed, U256::from(99), "parsed value must be 99");
        assert_eq!(
            ret.chainId,
            U256::from(99),
            "chainId must match parsed value"
        );
    }

    // -----------------------------------------------------------------
    // Cross-transaction determinism
    // -----------------------------------------------------------------

    /// vm.parseUint with the same string must return the same value when
    /// called in a separate transaction after the initial setup.
    #[test]
    fn parse_uint_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = ParseTarget::actionParseUintCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionParseUint must succeed");
        let stored = call_uint256_getter!(&mut chain, target, ParseTarget::getParsedUintCall);
        assert_eq!(
            stored, EXPECTED_UINT,
            "stored uint must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "actionParseUint must succeed on second call"
        );
        let stored = call_uint256_getter!(&mut chain, target, ParseTarget::getParsedUintCall);
        assert_eq!(
            stored, EXPECTED_UINT,
            "stored uint must still match after second action"
        );
    }
}
