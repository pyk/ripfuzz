//! `getEnv` cheatcodes - read environment variables as strings.

use alloy_dyn_abi::DynSolValue;

use crate::evm::cheatcode::outcome;

fn encode_string(value: &str) -> revm::interpreter::CallOutcome {
    let encoded = DynSolValue::String(value.to_owned()).abi_encode();
    outcome::success_bytes(encoded)
}

fn missing_env_error(key: &str) -> revm::interpreter::CallOutcome {
    outcome::revert(&format!(
        "Failed to get environment variable {key} as type string: environment variable not found"
    ))
}

/// Read an environment variable. Reverts if the key is not defined.
pub fn get_env(key: &str) -> Option<revm::interpreter::CallOutcome> {
    match std::env::var(key) {
        Ok(value) => Some(encode_string(&value)),
        Err(std::env::VarError::NotPresent) => Some(missing_env_error(key)),
        Err(std::env::VarError::NotUnicode(_)) => Some(outcome::revert(&format!(
            "Failed to get environment variable {key} as type string: environment variable was not valid unicode"
        ))),
    }
}

/// Read an environment variable, or return `default_value` if the key is not defined.
pub fn get_env_or_default(
    key: &str,
    default_value: &str,
) -> Option<revm::interpreter::CallOutcome> {
    match std::env::var(key) {
        Ok(value) => Some(encode_string(&value)),
        Err(std::env::VarError::NotPresent) => Some(encode_string(default_value)),
        Err(std::env::VarError::NotUnicode(_)) => Some(outcome::revert(&format!(
            "Failed to get environment variable {key} as type string: environment variable was not valid unicode"
        ))),
    }
}

#[cfg(test)]
mod tests {

    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::Address;
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::compilers::solc::{Solc, SolcOutput};
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::get_env;
    use crate::harness::HarnessId;

    fn compile_fixture(root: &str, target: &str) -> SolcOutput {
        let id = HarnessId::try_from(target).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        Solc::new()
            .with_version("0.8.36")
            .with_root(root)
            .with_target(&id.path)
            .with_name(&id.name)
            .with_out(tmp.path().join("out"))
            .compile()
            .unwrap()
    }

    alloy_sol_types::sol! {
        interface GetEnvHarness {
            function setup() external;
            function actionGetEnvOrDefault() external;
            function actionMutateViaDefault() external;
            function actionGetEnvMissing() external;
            function getStoredValue() external view returns (string memory);
            function getEnvDirect(string calldata key) external returns (string memory);
            function getEnvOrDefaultDirect(string calldata key, string calldata defaultValue)
                external
                returns (string memory);
            function invariant_getEnv() external view;
        }
    }

    const DEFINED_KEY: &str = "PATH";
    const EXPECTED_DEFAULT: &str = "default-value";
    const MISSING_KEY: &str = "RIPFUZZ_TEST_GET_ENV_MISSING_XYZ";

    fn decode_string_outcome(outcome: &revm::interpreter::CallOutcome) -> String {
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .expect("return data must decode as string");
        match decoded {
            DynSolValue::String(s) => s,
            other => panic!("expected string return data, got {other:?}"),
        }
    }

    fn load_initcode(id: &str) -> String {
        compile_fixture("fixtures/evm/cheatcodes", id)
            .initcode()
            .unwrap()
            .to_owned()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let initcode = load_initcode("GetEnvHarness.sol:GetEnvHarness");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// vm.getEnv must return the value of a defined environment variable.
    #[test]
    fn get_env_returns_defined_value() {
        let expected =
            std::env::var(DEFINED_KEY).expect("PATH must be set in the test environment");
        let outcome = get_env::get_env(DEFINED_KEY).expect("must return an outcome");
        assert!(
            outcome.result.is_ok(),
            "vm.getEnv must succeed for defined key"
        );
        assert_eq!(
            decode_string_outcome(&outcome),
            expected,
            "vm.getEnv must return the environment value"
        );
    }

    /// vm.getEnv must revert when the environment variable is not defined.
    #[test]
    fn get_env_missing_key_reverts() {
        let outcome = get_env::get_env(MISSING_KEY).expect("must return an outcome");
        assert!(
            !outcome.result.is_ok(),
            "vm.getEnv must revert for missing key"
        );
    }

    /// vm.getEnv with a default must return the environment value when defined.
    #[test]
    fn get_env_or_default_returns_defined_value() {
        let expected =
            std::env::var(DEFINED_KEY).expect("PATH must be set in the test environment");
        let outcome =
            get_env::get_env_or_default(DEFINED_KEY, "fallback").expect("must return an outcome");
        assert!(
            outcome.result.is_ok(),
            "vm.getEnv with default must succeed for defined key"
        );
        assert_eq!(
            decode_string_outcome(&outcome),
            expected,
            "vm.getEnv with default must prefer the environment value"
        );
    }

    /// vm.getEnv with a default must return the default when the key is missing.
    #[test]
    fn get_env_or_default_returns_default_for_missing_key() {
        let outcome =
            get_env::get_env_or_default(MISSING_KEY, "fallback").expect("must return an outcome");
        assert!(
            outcome.result.is_ok(),
            "vm.getEnv with default must succeed for missing key"
        );
        assert_eq!(
            decode_string_outcome(&outcome),
            "fallback",
            "vm.getEnv with default must return the default value"
        );
    }

    /// `vm.getEnv` used during setup must store the default-seeded value so that
    /// a later invariant call can verify the canonical value.
    #[test]
    fn get_env_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::getStoredValueCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::invariant_getEnvCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "getStoredValue must return the setup value"
        );
        let stored = GetEnvHarness::getStoredValueCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(stored, EXPECTED_DEFAULT, "stored value must match expected");
        assert!(
            execution.results[1].success,
            "invariant must pass after setup"
        );
    }

    /// Re-running `vm.getEnv` with a default in a later transaction must restore
    /// the canonical value.
    #[test]
    fn restore_get_env_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionGetEnvOrDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::invariant_getEnvCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionGetEnvOrDefault must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring value"
        );
    }

    /// Mutating the stored value via the default overload and then restoring
    /// it must leave the invariant intact.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionMutateViaDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionGetEnvOrDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::invariant_getEnvCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateViaDefault must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionGetEnvOrDefault must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// The single-argument overload must revert when the key is missing.
    #[test]
    fn get_env_missing_in_action_reverts() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            GetEnvHarness::actionGetEnvMissingCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "actionGetEnvMissing must revert"
        );
    }

    /// Direct `vm.getEnv` calls from the harness must return a defined
    /// environment value without relying on stored state.
    #[test]
    fn get_env_direct_returns_environment_value() {
        let expected =
            std::env::var(DEFINED_KEY).expect("PATH must be set in the test environment");
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            GetEnvHarness::getEnvDirectCall::new((DEFINED_KEY.to_string(),)).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(execution.results[0].success, "getEnvDirect must succeed");
        let value = GetEnvHarness::getEnvDirectCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(value, expected);
    }

    /// Direct `vm.getEnv` with default must return the default for a missing key.
    #[test]
    fn get_env_or_default_direct_returns_default() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::getEnvOrDefaultDirectCall::new((
                    MISSING_KEY.to_string(),
                    EXPECTED_DEFAULT.to_string(),
                ))
                .abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "getEnvOrDefaultDirect must succeed"
        );
        let value = GetEnvHarness::getEnvOrDefaultDirectCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(value, EXPECTED_DEFAULT);
    }

    /// Direct `vm.getEnv` with default must prefer a defined environment value.
    #[test]
    fn get_env_or_default_direct_prefers_environment_value() {
        let expected =
            std::env::var(DEFINED_KEY).expect("PATH must be set in the test environment");
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::getEnvOrDefaultDirectCall::new((
                    DEFINED_KEY.to_string(),
                    "fallback".to_string(),
                ))
                .abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "getEnvOrDefaultDirect must succeed"
        );
        let value = GetEnvHarness::getEnvOrDefaultDirectCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(value, expected);
    }

    /// A cloned chain snapshot must produce the same getEnv-derived state when
    /// actions are executed on the clone.
    #[test]
    fn cloned_chain_preserves_get_env() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionGetEnvOrDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::invariant_getEnvCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionGetEnvOrDefault must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate contract state via
    /// getEnv overloads, and a final invariant verifies the canonical value.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionGetEnvOrDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionMutateViaDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::actionGetEnvOrDefaultCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                GetEnvHarness::invariant_getEnvCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all sequence steps must succeed"
        );
    }
}
