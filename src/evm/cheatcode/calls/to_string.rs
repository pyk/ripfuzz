//! `toString` cheatcodes - convert Solidity values to strings.

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::I256;
use revm::primitives::{Address, Bytes, U256};

use crate::evm::cheatcode::outcome;

fn to_string_outcome(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let encoded = DynSolValue::String(s.to_owned()).abi_encode();
    Some(outcome::success_bytes(encoded))
}

pub fn to_string_address(addr: Address) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{addr}");
    to_string_outcome(&s)
}

pub fn to_string_bool(b: bool) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{b}");
    to_string_outcome(&s)
}

pub fn to_string_uint(value: U256) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{value}");
    to_string_outcome(&s)
}

pub fn to_string_int(value: I256) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{value}");
    to_string_outcome(&s)
}

pub fn to_string_bytes32(b: [u8; 32]) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s)
}

pub fn to_string_bytes(b: Bytes) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s)
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{Address, I256, U256, address};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployOptions, SetupOptions};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::to_string;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface ToStringTarget {
            function getStoredAddrString() external view returns (string memory s);
            function getStoredBoolString() external view returns (string memory s);
            function getStoredUintString() external view returns (string memory s);
            function getStoredIntString() external view returns (string memory s);
            function getStoredBytes32String() external view returns (string memory s);
            function getStoredBytesString() external view returns (string memory s);

            function getAddrZeroString() external pure returns (string memory s);
            function getBoolFalseString() external pure returns (string memory s);
            function getUintZeroString() external pure returns (string memory s);
            function getIntZeroString() external pure returns (string memory s);
            function getIntNegativeOneString() external pure returns (string memory s);
            function getIntMinString() external pure returns (string memory s);
            function getUintMaxString() external pure returns (string memory s);
            function getBytes32ZeroString() external pure returns (string memory s);
            function getBytesEmptyString() external pure returns (string memory s);

            function callToStringAddrSameValueTwice() external pure returns (string memory first, string memory second);
            function callToStringBoolSameValueTwice() external pure returns (string memory first, string memory second);
            function callToStringUintSameValueTwice() external pure returns (string memory first, string memory second);
            function callToStringIntSameValueTwice() external pure returns (string memory first, string memory second);
            function callToStringBytes32SameValueTwice() external pure returns (string memory first, string memory second);
            function callToStringBytesSameValueTwice() external pure returns (string memory first, string memory second);

            function callToStringUintSequence() external pure returns (string memory first, string memory second, string memory third);
            function callToStringBoolSequence() external pure returns (string memory first, string memory second, string memory third);

            function callToStringAndWarp() external pure returns (string memory addrStr, uint256 timestamp);
            function callToStringAndDeal() external returns (string memory addrStr, uint256 balance);

            function setup() external;

            function actionToStringAddr() external;
            function actionToStringBool() external;
            function actionToStringUint() external;
            function actionToStringInt() external;
            function actionToStringBytes32() external;
            function actionToStringBytes() external;

            function invariant_to_string_addr() external view;
            function invariant_to_string_bool() external view;
            function invariant_to_string_uint() external view;
            function invariant_to_string_int() external view;
            function invariant_to_string_bytes32() external view;
            function invariant_to_string_bytes() external view;
        }
    }

    const TEST_ADDR: Address = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
    const TEST_BYTES32: [u8; 32] = [
        0xab, 0xcd, 0xef, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
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
        let contract = load_fixture("src/ToStringTarget.sol:ToStringTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployOptions::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(ToStringTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupOptions::new(target, setup_data);
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

    /// Call a view/pure function that returns a single `string` and decode it.
    macro_rules! call_string_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
        ($chain:expr, $target:expr, $call:ty, $args:tt) => {{
            let calldata = <$call>::new($args).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn decode_string_outcome(outcome: &revm::interpreter::CallOutcome) -> String {
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .unwrap();
        let DynSolValue::String(s) = decoded else {
            panic!("decoded value must be a string")
        };
        s
    }

    // -----------------------------------------------------------------
    // Handler-level (direct Rust unit tests)
    // -----------------------------------------------------------------

    /// vm.toString(address) must return the checksummed address string.
    #[test]
    fn to_string_address_returns_checksummed_hex() {
        let outcome = to_string::to_string_address(TEST_ADDR);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(address) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(
            decoded, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "address must be checksummed"
        );
    }

    /// vm.toString(bool) must return "true" for true.
    #[test]
    fn to_string_bool_true_returns_true() {
        let outcome = to_string::to_string_bool(true);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(true) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "true");
    }

    /// vm.toString(bool) must return "false" for false.
    #[test]
    fn to_string_bool_false_returns_false() {
        let outcome = to_string::to_string_bool(false);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(false) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "false");
    }

    /// vm.toString(uint256) must return decimal without leading zeros.
    #[test]
    fn to_string_uint_returns_decimal() {
        let outcome = to_string::to_string_uint(U256::from(42));
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(uint) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "42");
    }

    /// vm.toString(uint256) edge case: zero must return "0".
    #[test]
    fn to_string_uint_zero_returns_zero() {
        let outcome = to_string::to_string_uint(U256::ZERO);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(0) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0");
    }

    /// vm.toString(uint256) edge case: max uint256 must return full decimal.
    #[test]
    fn to_string_uint_max_returns_full_decimal() {
        let outcome = to_string::to_string_uint(U256::MAX);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(MAX) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(
            decoded,
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
    }

    /// vm.toString(int256) must return decimal with minus sign for negatives.
    #[test]
    fn to_string_int_negative_returns_signed_decimal() {
        let value = -I256::from_raw(U256::from(42));
        let outcome = to_string::to_string_int(value);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(int) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "-42");
    }

    /// vm.toString(int256) edge case: zero must return "0".
    #[test]
    fn to_string_int_zero_returns_zero() {
        let outcome = to_string::to_string_int(I256::ZERO);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(int(0)) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0");
    }

    /// vm.toString(int256) edge case: min int256 must return full signed decimal.
    #[test]
    fn to_string_int_min_returns_full_decimal() {
        let outcome = to_string::to_string_int(I256::MIN);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            outcome.result.is_ok(),
            "vm.toString(int256::MIN) must succeed"
        );
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(
            decoded,
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968"
        );
    }

    /// vm.toString(bytes32) must return lowercase hex with 0x prefix.
    #[test]
    fn to_string_bytes32_returns_hex() {
        let outcome = to_string::to_string_bytes32(TEST_BYTES32);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(bytes32) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(
            decoded,
            "0xabcdef0000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// vm.toString(bytes32) edge case: zero bytes32 must return 64 zeros.
    #[test]
    fn to_string_bytes32_zero_returns_zeros() {
        let outcome = to_string::to_string_bytes32([0u8; 32]);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            outcome.result.is_ok(),
            "vm.toString(bytes32(0)) must succeed"
        );
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(
            decoded,
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// vm.toString(bytes) must return lowercase hex with 0x prefix.
    #[test]
    fn to_string_bytes_returns_hex() {
        let data = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]);
        let outcome = to_string::to_string_bytes(data);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(bytes) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0xdeadbeef");
    }

    /// vm.toString(bytes) edge case: empty bytes must return "0x".
    #[test]
    fn to_string_bytes_empty_returns_0x() {
        let data = Bytes::new();
        let outcome = to_string::to_string_bytes(data);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            outcome.result.is_ok(),
            "vm.toString(empty bytes) must succeed"
        );
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0x");
    }

    // -----------------------------------------------------------------
    // Basic contract-path integration
    // -----------------------------------------------------------------

    /// All toString values stored during setup must match expected strings.
    #[test]
    fn to_string_values_persist_after_setup() {
        let (mut chain, target) = deploy_and_setup();

        let ret = call_string_getter!(&mut chain, target, ToStringTarget::getStoredAddrStringCall);
        assert_eq!(ret, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");

        let ret = call_string_getter!(&mut chain, target, ToStringTarget::getStoredBoolStringCall);
        assert_eq!(ret, "true");

        let ret = call_string_getter!(&mut chain, target, ToStringTarget::getStoredUintStringCall);
        assert_eq!(ret, "12345678901234567890");

        let ret = call_string_getter!(&mut chain, target, ToStringTarget::getStoredIntStringCall);
        assert_eq!(ret, "-12345678901234567890");

        let ret = call_string_getter!(
            &mut chain,
            target,
            ToStringTarget::getStoredBytes32StringCall
        );
        assert_eq!(
            ret,
            "0xabcdef0000000000000000000000000000000000000000000000000000000000"
        );

        let ret = call_string_getter!(&mut chain, target, ToStringTarget::getStoredBytesStringCall);
        assert_eq!(ret, "0xdeadbeef");
    }

    /// Edge case: toString(address(0)) must return 40 zeros with 0x prefix.
    #[test]
    fn to_string_addr_zero_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getAddrZeroStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getAddrZeroString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getAddrZeroStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "0x0000000000000000000000000000000000000000");
    }

    /// Edge case: toString(false) must return "false".
    #[test]
    fn to_string_bool_false_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getBoolFalseStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getBoolFalseString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getBoolFalseStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "false");
    }

    /// Edge case: toString(uint256(0)) must return "0".
    #[test]
    fn to_string_uint_zero_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getUintZeroStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getUintZeroString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getUintZeroStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "0");
    }

    /// Edge case: toString(int256(0)) must return "0".
    #[test]
    fn to_string_int_zero_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getIntZeroStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getIntZeroString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getIntZeroStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "0");
    }

    /// Edge case: toString(int256(-1)) must return "-1".
    #[test]
    fn to_string_int_negative_one_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getIntNegativeOneStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getIntNegativeOneString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getIntNegativeOneStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "-1");
    }

    /// Edge case: toString(type(int256).min) must return full signed decimal.
    #[test]
    fn to_string_int_min_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getIntMinStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getIntMinString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getIntMinStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret,
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968"
        );
    }

    /// Edge case: toString(type(uint256).max) must return full decimal.
    #[test]
    fn to_string_uint_max_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getUintMaxStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getUintMaxString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getUintMaxStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret,
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
    }

    /// Edge case: toString(bytes32(0)) must return 64 zeros.
    #[test]
    fn to_string_bytes32_zero_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getBytes32ZeroStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getBytes32ZeroString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getBytes32ZeroStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret,
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// Edge case: toString(empty bytes) must return "0x".
    #[test]
    fn to_string_bytes_empty_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::getBytesEmptyStringCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getBytesEmptyString() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::getBytesEmptyStringCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, "0x");
    }

    // -----------------------------------------------------------------
    // Single-transaction determinism / sequence
    // -----------------------------------------------------------------

    /// vm.toString(address) called twice in one tx must return identical strings.
    #[test]
    fn to_string_addr_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringAddrSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringAddrSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringAddrSameValueTwiceCall::abi_decode_returns(&output)
            .unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same address must give identical toString"
        );
        assert_eq!(ret.first, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
    }

    /// vm.toString(bool) called twice in one tx must return identical strings.
    #[test]
    fn to_string_bool_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringBoolSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringBoolSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringBoolSameValueTwiceCall::abi_decode_returns(&output)
            .unwrap();
        assert_eq!(ret.first, ret.second);
        assert_eq!(ret.first, "true");
    }

    /// vm.toString(uint256) called twice in one tx must return identical strings.
    #[test]
    fn to_string_uint_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringUintSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringUintSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringUintSameValueTwiceCall::abi_decode_returns(&output)
            .unwrap();
        assert_eq!(ret.first, ret.second);
        assert_eq!(ret.first, "12345678901234567890");
    }

    /// vm.toString(int256) called twice in one tx must return identical strings.
    #[test]
    fn to_string_int_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringIntSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringIntSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret =
            ToStringTarget::callToStringIntSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, ret.second);
        assert_eq!(ret.first, "-12345678901234567890");
    }

    /// vm.toString(bytes32) called twice in one tx must return identical strings.
    #[test]
    fn to_string_bytes32_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringBytes32SameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringBytes32SameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret =
            ToStringTarget::callToStringBytes32SameValueTwiceCall::abi_decode_returns(&output)
                .unwrap();
        assert_eq!(ret.first, ret.second);
        assert_eq!(
            ret.first,
            "0xabcdef0000000000000000000000000000000000000000000000000000000000"
        );
    }

    /// vm.toString(bytes) called twice in one tx must return identical strings.
    #[test]
    fn to_string_bytes_same_value_twice_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringBytesSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callToStringBytesSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringBytesSameValueTwiceCall::abi_decode_returns(&output)
            .unwrap();
        assert_eq!(ret.first, ret.second);
        assert_eq!(ret.first, "0xdeadbeef");
    }

    /// vm.toString(uint256) with different values interleaved must produce
    /// distinct strings, proving sequence independence.
    #[test]
    fn to_string_uint_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringUintSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callToStringUintSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            ToStringTarget::callToStringUintSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, "1", "first vm.toString(1) must read 1");
        assert_eq!(
            ret.second, "12345678901234567890",
            "second vm.toString(TEST_UINT) must read correctly"
        );
        assert_eq!(ret.third, "1", "third vm.toString(1) must read 1 again");
    }

    /// vm.toString(bool) with different values interleaved must produce
    /// distinct strings, proving sequence independence.
    #[test]
    fn to_string_bool_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringBoolSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callToStringBoolSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            ToStringTarget::callToStringBoolSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, "true");
        assert_eq!(ret.second, "false");
        assert_eq!(ret.third, "true");
    }

    // -----------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------

    /// Strings converted during setup must still match when read in later
    /// transactions, proving contract-level persistence works.
    #[test]
    fn to_string_setup_values_persist_in_storage() {
        let (mut chain, target) = deploy_and_setup();

        let first =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredAddrStringCall);
        let second =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredAddrStringCall);
        assert_eq!(first, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        assert_eq!(
            first, second,
            "getter must return same stored string across calls"
        );

        let first =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredUintStringCall);
        let second =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredUintStringCall);
        assert_eq!(first, "12345678901234567890");
        assert_eq!(first, second);
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
                ToStringTarget::invariant_to_string_addrCall::new(()).abi_encode(),
                "invariant_to_string_addr",
            ),
            (
                ToStringTarget::invariant_to_string_boolCall::new(()).abi_encode(),
                "invariant_to_string_bool",
            ),
            (
                ToStringTarget::invariant_to_string_uintCall::new(()).abi_encode(),
                "invariant_to_string_uint",
            ),
            (
                ToStringTarget::invariant_to_string_intCall::new(()).abi_encode(),
                "invariant_to_string_int",
            ),
            (
                ToStringTarget::invariant_to_string_bytes32Call::new(()).abi_encode(),
                "invariant_to_string_bytes32",
            ),
            (
                ToStringTarget::invariant_to_string_bytesCall::new(()).abi_encode(),
                "invariant_to_string_bytes",
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
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Mutate strings via actions then verify invariants.
        let actions = [
            (
                ToStringTarget::actionToStringAddrCall::new(()).abi_encode(),
                "actionToStringAddr",
            ),
            (
                ToStringTarget::actionToStringBoolCall::new(()).abi_encode(),
                "actionToStringBool",
            ),
            (
                ToStringTarget::actionToStringUintCall::new(()).abi_encode(),
                "actionToStringUint",
            ),
            (
                ToStringTarget::actionToStringIntCall::new(()).abi_encode(),
                "actionToStringInt",
            ),
            (
                ToStringTarget::actionToStringBytes32Call::new(()).abi_encode(),
                "actionToStringBytes32",
            ),
            (
                ToStringTarget::actionToStringBytesCall::new(()).abi_encode(),
                "actionToStringBytes",
            ),
        ];

        for (calldata, name) in actions {
            let result = call_with_cheatcode_inspector(
                &mut chain,
                DEFAULT_DEPLOYER,
                target,
                Bytes::from(calldata),
            );
            assert!(result.success, "{name} must succeed");
        }

        let invariants = [
            (
                ToStringTarget::invariant_to_string_addrCall::new(()).abi_encode(),
                "invariant_to_string_addr",
            ),
            (
                ToStringTarget::invariant_to_string_boolCall::new(()).abi_encode(),
                "invariant_to_string_bool",
            ),
            (
                ToStringTarget::invariant_to_string_uintCall::new(()).abi_encode(),
                "invariant_to_string_uint",
            ),
            (
                ToStringTarget::invariant_to_string_intCall::new(()).abi_encode(),
                "invariant_to_string_int",
            ),
            (
                ToStringTarget::invariant_to_string_bytes32Call::new(()).abi_encode(),
                "invariant_to_string_bytes32",
            ),
            (
                ToStringTarget::invariant_to_string_bytesCall::new(()).abi_encode(),
                "invariant_to_string_bytes",
            ),
        ];

        for (calldata, name) in invariants {
            let result = chain
                .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{name} must pass after action sequence");
        }
    }

    /// vm.toString must stay deterministic across multiple transactions.
    #[test]
    fn to_string_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = ToStringTarget::actionToStringUintCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionToStringUint must succeed");
        let stored =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredUintStringCall);
        assert_eq!(
            stored, "12345678901234567890",
            "stored uint string must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "actionToStringUint must succeed on second call"
        );
        let stored =
            call_string_getter!(&mut chain, target, ToStringTarget::getStoredUintStringCall);
        assert_eq!(
            stored, "12345678901234567890",
            "stored uint string must still match after second action"
        );
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    /// vm.toString must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn to_string_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callToStringAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.addrStr, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "address string must match expected"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// vm.toString must work correctly when combined with vm.deal in the same tx.
    #[test]
    fn to_string_interacts_with_deal() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ToStringTarget::callToStringAndDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callToStringAndDeal() must succeed");
        let output = result.output.expect("must return output");
        let ret = ToStringTarget::callToStringAndDealCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.addrStr, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "address string must match expected"
        );
        assert_eq!(
            ret.balance,
            U256::from(5_000_000_000_000_000_000u64),
            "balance must match dealt value (5 ether)"
        );
    }
}
