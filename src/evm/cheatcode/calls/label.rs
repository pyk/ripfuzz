//! `label` / `getLabel` cheatcodes.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Address;

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn label(
    state: &mut ExecutionState,
    addr: Address,
    name: &str,
) -> Option<revm::interpreter::CallOutcome> {
    state.labels.insert(addr, name.into());
    Some(outcome::success())
}

pub fn get_label(
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let name = state.labels.get(&addr).cloned().unwrap_or_default();
    let encoded = DynSolValue::String(name).abi_encode();
    Some(outcome::success_bytes(encoded))
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{Address, address};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::label;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface LabelTarget {
            function setup() external;
            function actionRelabelAdmin() external;
            function actionRestoreLabels() external;
            function actionOverwriteAdmin() external;
            function actionRelabelUser() external;
            function actionRestoreUser() external;
            function getAdminLabelDirect() external view returns (string memory);
            function invariant_labelsMatch() external view;
        }
    }

    const ADMIN: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const ADMIN_LABEL: &str = "admin";

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/LabelTarget.sol:LabelTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // -----------------------------------------------------------------------
    // Handler-level unit tests
    // -----------------------------------------------------------------------

    /// vm.label must store the name in execution state.
    #[test]
    fn label_stores_name_in_execution_state() {
        let mut state = ExecutionState::default();
        let outcome = label::label(&mut state, ADMIN, ADMIN_LABEL);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.label must succeed");
        assert_eq!(
            state.labels.get(&ADMIN),
            Some(&ADMIN_LABEL.into()),
            "label must be stored in execution state"
        );
    }

    /// vm.getLabel must return the stored name for a labeled address.
    #[test]
    fn get_label_returns_stored_name() {
        let mut state = ExecutionState::default();
        state.labels.insert(ADMIN, ADMIN_LABEL.into());
        let outcome = label::get_label(&mut state, ADMIN);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getLabel must succeed");
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(ADMIN_LABEL.into()),
            "vm.getLabel must return the stored name"
        );
    }

    /// vm.getLabel on an unlabeled address must return an empty string.
    #[test]
    fn get_label_unknown_address_returns_empty() {
        let mut state = ExecutionState::default();
        let outcome = label::get_label(&mut state, ADMIN);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.getLabel must succeed");
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(String::new()),
            "vm.getLabel on unknown address must return empty string"
        );
    }

    /// vm.label must overwrite an existing label for the same address.
    #[test]
    fn label_overwrites_existing_name() {
        let mut state = ExecutionState::default();
        label::label(&mut state, ADMIN, "old");
        label::label(&mut state, ADMIN, ADMIN_LABEL);
        let outcome = label::get_label(&mut state, ADMIN);
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.unwrap().result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(ADMIN_LABEL.into()),
            "vm.label must overwrite the previous label"
        );
    }

    /// vm.label with an empty string must store the empty label.
    #[test]
    fn label_empty_string_is_allowed() {
        let mut state = ExecutionState::default();
        let outcome = label::label(&mut state, ADMIN, "");
        assert!(outcome.is_some(), "must return an outcome");
        assert!(outcome.unwrap().result.is_ok(), "vm.label('') must succeed");
        let outcome = label::get_label(&mut state, ADMIN);
        let string_type = DynSolType::String;
        let decoded = string_type
            .abi_decode_params(&outcome.unwrap().result.output)
            .unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(String::new()),
            "vm.getLabel must return empty string after vm.label('')"
        );
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    /// `vm.label` used during setup must persist the stored labels so that
    /// a later invariant call can verify the canonical values.
    #[test]
    fn labels_set_in_setup_match_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// Re-labeling an address in an action without restoring it must break
    /// the invariant, proving the cheatcode state is actually mutated.
    #[test]
    fn relabel_admin_in_action_fails_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRelabelAdmin must succeed"
        );
        assert!(
            !execution.results[1].success,
            "invariant must fail after relabeling admin"
        );
    }

    /// Re-labeling an address and then restoring the canonical label in the
    /// same sequence must leave the invariant intact.
    #[test]
    fn restore_labels_in_action_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelUserCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRestoreLabelsCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(
            execution.results[0].success,
            "actionRelabelAdmin must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionRelabelUser must succeed"
        );
        assert!(
            execution.results[2].success,
            "actionRestoreLabels must succeed"
        );
        assert!(
            execution.results[3].success,
            "invariant must pass after restoring labels"
        );
    }

    /// Overwriting a label multiple times in a single transaction and ending
    /// on the canonical value must keep the invariant intact. This proves
    /// the cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn overwrite_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionOverwriteAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionOverwriteAdmin must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after overwrite sequence"
        );
    }

    /// A cloned chain snapshot must produce the same label state when the
    /// invariant is executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_label_state() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRestoreLabelsCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionRelabelAdmin must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "actionRestoreLabels must succeed on cloned chain"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass on cloned chain"
        );
    }

    /// `vm.label` set during setup must be visible to `vm.getLabel` in a
    /// later `chain.exec` call without re-labeling. This proves that the
    /// cheatcode inspector state is properly snapshotted after setup.
    #[test]
    fn label_persists_from_setup_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            LabelTarget::getAdminLabelDirectCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "getAdminLabelDirect must succeed"
        );
        let ret = LabelTarget::getAdminLabelDirectCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            ret, ADMIN_LABEL,
            "vm.getLabel must return the label set during setup"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate label state by
    /// re-labeling different addresses, and a final invariant verifies that
    /// the canonical values are still intact.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRelabelUserCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionRestoreLabelsCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::actionOverwriteAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                LabelTarget::invariant_labelsMatchCall::new(()).abi_encode(),
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
