//! `ffi` cheatcode - execute arbitrary host commands.

use std::process::Command;

use alloy_dyn_abi::DynSolValue;

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle(
    args: Vec<String>,

    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    if !state.ffi_enabled {
        return Some(outcome::revert("ffi disabled: use --ffi to enable"));
    }
    if args.is_empty() {
        return Some(outcome::revert("ffi: empty command"));
    }

    let output = match run_ffi(&args, &state.project_root) {
        Ok(out) => out,
        Err(e) => return Some(outcome::revert(&e)),
    };
    let encoded = DynSolValue::Bytes(output).abi_encode();
    Some(outcome::success_bytes(encoded))
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

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::ffi;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface FfiTarget {
            function getValue() external view returns (uint256);
            function callFfiSameValueTwice() external returns (uint256 first, uint256 second);
            function callFfiSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callFfiAndWarp() external returns (uint256 value, uint256 timestamp);
            function setup() external;
            function actionFfi() external;
            function invariant_ffi() external view;
        }
    }

    const EXPECTED_VALUE: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Create an inspector with ffi enabled and a valid project root.
    fn ffi_enabled_inspector() -> cheatcode::Inspector {
        let config =
            cheatcode::Config::new(std::env::current_dir().unwrap_or_default()).with_ffi(true);
        cheatcode::Inspector::new(config)
    }

    /// Deploy the fixture and run its `setup` function with an FFI-enabled inspector.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/FfiTarget.sol:FfiTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(FfiTarget::setupCall::new(()).abi_encode());
        let tx = revm::context::TxEnv {
            caller: DEFAULT_DEPLOYER,
            kind: revm::primitives::TxKind::Call(target),
            data: setup_data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        let inspector = ffi_enabled_inspector();
        let (result, _) = chain.inspect(tx, inspector).unwrap();
        assert!(result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with an FFI-enabled cheatcode inspector.
    fn call_with_ffi_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = ffi_enabled_inspector();
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

    /// vm.ffi must revert when ffi is disabled.
    #[test]
    fn ffi_disabled_reverts() {
        let mut state = ExecutionState::default();
        let outcome = ffi::handle(vec!["echo".to_string()], &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(!outcome.result.is_ok(), "vm.ffi must revert when disabled");
    }

    /// vm.ffi must return decoded bytes when ffi is enabled.
    #[test]
    fn ffi_enabled_returns_output() {
        let mut state = ExecutionState::default();
        state.ffi_enabled = true;
        state.project_root = std::env::current_dir().unwrap_or_default();
        let outcome = ffi::handle(
            vec![
                "printf".to_string(),
                "%s".to_string(),
                "000000000000000000000000000000000000000000000000000000000000002a".to_string(),
            ],
            &mut state,
        );
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.ffi must succeed when enabled");
    }

    /// vm.ffi with empty args must revert.
    #[test]
    fn ffi_empty_args_reverts() {
        let mut state = ExecutionState::default();
        state.ffi_enabled = true;
        let outcome = ffi::handle(vec![], &mut state);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.ffi with empty args must revert"
        );
    }

    /// The value obtained via vm.ffi during setup must be readable via the
    /// contract getter in a later transaction.
    #[test]
    fn ffi_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 = call_uint256_getter!(&mut chain, target, FfiTarget::getValueCall);
        assert_eq!(
            decoded, EXPECTED_VALUE,
            "ffi result must persist in contract storage"
        );
    }

    /// vm.ffi with the same args twice in one tx must yield the same
    /// reading, proving determinism.
    #[test]
    fn ffi_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FfiTarget::callFfiSameValueTwiceCall::new(()).abi_encode();
        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "callFfiSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = FfiTarget::callFfiSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same ffi args must give identical readings"
        );
        assert_eq!(ret.first, EXPECTED_VALUE);
    }

    /// vm.ffi with different args interleaved must produce distinct
    /// readings, proving the cheatcode responds to different inputs.
    #[test]
    fn ffi_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FfiTarget::callFfiSequenceCall::new(()).abi_encode();
        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "callFfiSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = FfiTarget::callFfiSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first ffi must read 1");
        assert_eq!(ret.second, EXPECTED_VALUE, "second ffi must read 42");
        assert_eq!(ret.third, U256::from(5), "third ffi must read 5");
    }

    /// vm.ffi must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn ffi_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FfiTarget::callFfiAndWarpCall::new(()).abi_encode();
        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "callFfiAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = FfiTarget::callFfiAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.value, EXPECTED_VALUE, "ffi value must match expected");
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FfiTarget::invariant_ffiCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate value via a sequence that ends on a different value.
        let calldata = FfiTarget::callFfiSequenceCall::new(()).abi_encode();
        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "callFfiSequence must succeed");

        // Restore the expected value with an action.
        let calldata = FfiTarget::actionFfiCall::new(()).abi_encode();
        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "actionFfi must succeed");

        // Invariant must pass after the action restored state.
        let calldata = FfiTarget::invariant_ffiCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.ffi(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn ffi_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = FfiTarget::actionFfiCall::new(()).abi_encode();

        let result = call_with_ffi_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionFfi must succeed");
        let stored: U256 = call_uint256_getter!(&mut chain, target, FfiTarget::getValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must match after first action"
        );

        let result =
            call_with_ffi_inspector(&mut chain, DEFAULT_DEPLOYER, target, Bytes::from(calldata));
        assert!(result.success, "actionFfi must succeed on second call");
        let stored: U256 = call_uint256_getter!(&mut chain, target, FfiTarget::getValueCall);
        assert_eq!(
            stored, EXPECTED_VALUE,
            "stored value must still match after second action"
        );
    }
}
