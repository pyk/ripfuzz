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

    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::ffi;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface FfiTarget {
            function setup() external;
            function actionFfi() external;
            function actionMutateFfi() external;
            function actionFfiSequence()
                external
                returns (uint256 first, uint256 second, uint256 third);
            function getValue() external view returns (uint256);
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

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/FfiTarget.sol:FfiTarget");
        let mut config = Config::default();
        config.cheatcode.ffi = true;
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
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

    /// `vm.ffi` used during setup must persist its decoded result in contract
    /// storage. The invariant verifies the stored value matches the expected
    /// canonical value.
    #[test]
    fn ffi_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::getValueCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "getValue must return the ffi-derived value"
        );
        let stored: U256 = FfiTarget::getValueCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(stored, EXPECTED_VALUE, "stored value must match expected");
        assert!(
            execution.results[1].success,
            "invariant must pass after setup"
        );
    }

    /// Re-running the same `vm.ffi` command in a later transaction must
    /// restore the canonical value. This is the core property a stateful
    /// fuzzer relies on when actions need to recover expected state.
    #[test]
    fn restore_ffi_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::actionFfiCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionFfi must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring value"
        );
    }

    /// A single transaction can interleave multiple `vm.ffi` calls with
    /// different arguments and end on the expected value without corrupting
    /// state. This proves the cheatcode is deterministic and safe to call
    /// repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::actionFfiSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionFfiSequence must succeed"
        );
        let output = execution.results[0].output.clone().unwrap();
        let ret = FfiTarget::actionFfiSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first ffi must read 1");
        assert_eq!(ret.second, EXPECTED_VALUE, "second ffi must read 42");
        assert_eq!(ret.third, U256::from(5), "third ffi must read 5");
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating the stored value via a different `vm.ffi` result and then
    /// restoring it in a sequence must leave the invariant intact. This
    /// mirrors how a stateful fuzzer would explore state mutations and then
    /// recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::actionMutateFfiCall::new(()).abi_encode(),
            )),
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::actionFfiCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(execution.results[0].success, "actionMutateFfi must succeed");
        assert!(execution.results[1].success, "actionFfi must succeed");
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same ffi-derived state when
    /// actions are executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_ffi() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::actionFfiCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionFfi must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate contract state
    /// by changing the stored ffi-derived value, and a final invariant
    /// verifies that the canonical value is still intact. This mirrors how a
    /// stateful fuzzer would use `vm.ffi` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::actionFfiCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::actionMutateFfiCall::new(()).abi_encode(),
            )),
            Transaction::new(target)
                .calldata(Bytes::from(FfiTarget::actionFfiCall::new(()).abi_encode())),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::actionFfiSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FfiTarget::invariant_ffiCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 5);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all sequence steps must succeed"
        );
    }
}
