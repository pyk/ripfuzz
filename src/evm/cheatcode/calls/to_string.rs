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

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::to_string;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface ToStringTarget {
            function setup() external;
            function actionRefreshAll() external;
            function invariant_addr() external view;
            function invariant_bool() external view;
            function invariant_uint() external view;
            function invariant_int() external view;
            function invariant_bytes32() external view;
            function invariant_bytes() external view;
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

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/ToStringTarget.sol:ToStringTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

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
    // Handler-level unit tests
    // -----------------------------------------------------------------

    /// vm.toString(address) must return the checksummed address string.
    #[test]
    fn address_checksummed() {
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

    /// vm.toString(false) must return "false".
    #[test]
    fn bool_false_returns_false() {
        let outcome = to_string::to_string_bool(false);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(false) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "false");
    }

    /// vm.toString(uint256(0)) must return "0" without leading zeros.
    #[test]
    fn uint_zero_returns_zero() {
        let outcome = to_string::to_string_uint(U256::ZERO);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(0) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0");
    }

    /// vm.toString(uint256) must return decimal without leading zeros.
    #[test]
    fn uint_returns_decimal() {
        let outcome = to_string::to_string_uint(U256::from(42));
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(uint) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "42");
    }

    /// vm.toString(int256(0)) must return "0".
    #[test]
    fn int_zero_returns_zero() {
        let outcome = to_string::to_string_int(I256::ZERO);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(int(0)) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "0");
    }

    /// vm.toString(int256) must return decimal with minus sign for negatives.
    #[test]
    fn int_negative_returns_signed_decimal() {
        let value = -I256::from_raw(U256::from(42));
        let outcome = to_string::to_string_int(value);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.toString(int) must succeed");
        let decoded = decode_string_outcome(&outcome);
        assert_eq!(decoded, "-42");
    }

    /// vm.toString(bytes32) must return lowercase hex with 0x prefix.
    #[test]
    fn bytes32_returns_hex() {
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

    /// vm.toString(empty bytes) must return "0x".
    #[test]
    fn bytes_empty_returns_0x() {
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
    // Integration tests
    // -----------------------------------------------------------------

    /// `vm.toString` used during setup must store correctly formatted strings.
    /// The invariants verify that all six type variants match their expected
    /// textual representations.
    #[test]
    fn setup_derives_well_known_strings() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_addrCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_boolCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_uintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_intCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytes32Call::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytesCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 6);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all invariants must pass after setup"
        );
    }

    /// Re-converting all canonical values in a later transaction and
    /// overwriting storage must not change the stored strings. This is the
    /// core property a stateful fuzzer relies on when actions refresh labels.
    #[test]
    fn refresh_in_action_preserves_strings() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_addrCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_boolCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_uintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_intCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytes32Call::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytesCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 7);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all steps must pass after refresh"
        );
    }

    /// A cloned chain snapshot must produce the same strings when actions are
    /// executed on the clone. This is critical for parallel fuzzing where each
    /// worker starts from a cloned state.
    #[test]
    fn cloned_chain_produces_same_strings() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_addrCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_boolCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_uintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_intCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytes32Call::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytesCall::new(()).abi_encode(),
            )),
        ];
        let execution = cloned.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 7);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all steps must pass on cloned chain"
        );
    }

    /// Cross-transaction determinism: re-converting in a second `exec` must
    /// still produce the same strings, leaving all invariants intact.
    #[test]
    fn deterministic_across_separate_execs() {
        let (mut chain, target) = deploy_and_setup();

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_addrCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_boolCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_uintCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_intCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytes32Call::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ToStringTarget::invariant_bytesCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(ExecInput::new(txs.clone())).unwrap();
        assert_eq!(execution.results.len(), 7);
        assert!(
            execution.results.iter().all(|r| r.success),
            "first exec must succeed"
        );

        let execution = chain.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 7);
        assert!(
            execution.results.iter().all(|r| r.success),
            "second exec must succeed"
        );
    }
}
