//! `etch` cheatcode - set contract bytecode at an address.

use revm::{
    bytecode::Bytecode,
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr},
    primitives::{Address, Bytes},
};

use crate::evm::cheatcode::outcome;

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    code: Bytes,
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal_mut().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("cannot etch precompile address"));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let bytecode = Bytecode::new_raw_checked(code)
        .map_err(|e| format!("failed to create bytecode: {e}"))
        .ok()?;
    ctx.journal_mut().set_code(addr, bytecode);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::foundry;

    alloy_sol_types::sol! {
        interface EtchHarness {
            function setup() external;
            function invariant_etch() external view;
            function actionRestoreEtch() external;
            function actionMutateEtch() external;
            function actionEtchSequence() external;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/harness-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/EtchHarness.sol:EtchHarness");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.etch` used during setup must persist in the EVM account code.
    /// The invariant checks that the live etched contract returns the
    /// expected canonical value.
    #[test]
    fn etch_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            EtchHarness::invariant_etchCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_etch must pass after setup"
        );
    }

    /// Re-etching the canonical code in a later transaction must not
    /// corrupt the harness contract. This is the core property a stateful
    /// fuzzer relies on when actions need to restore canonical bytecode.
    #[test]
    fn restore_etch_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionRestoreEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::invariant_etchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreEtch must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring etch"
        );
    }

    /// A single transaction can interleave multiple `vm.etch` calls and
    /// end on the expected code without corrupting state. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionEtchSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::invariant_etchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionEtchSequence must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating etched code and then restoring it in a sequence must leave
    /// the invariant intact. This mirrors how a stateful fuzzer would
    /// explore state mutations and then recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionMutateEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionRestoreEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::invariant_etchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateEtch must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionRestoreEtch must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same etched code when
    /// actions are executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_etch() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionRestoreEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::invariant_etchCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreEtch must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate account code
    /// by etching different contracts, and a final invariant verifies that
    /// the canonical bytecode is still intact. This mirrors how a stateful
    /// fuzzer would use `vm.etch` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionRestoreEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionMutateEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionRestoreEtchCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::actionEtchSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                EtchHarness::invariant_etchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 5);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all sequence steps must succeed"
        );
    }
}
