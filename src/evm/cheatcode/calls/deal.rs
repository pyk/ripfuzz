//! `deal` cheatcode - set an account balance.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    value: U256,
) -> Option<revm::interpreter::CallOutcome> {
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_balance(value);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface DealTarget {
            function setup() external;
            function invariant_deal() external view;
            function actionRestoreDeal() external;
            function actionMutateDeal() external;
            function actionDealSequence() external;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/DealTarget.sol:DealTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.deal` used during setup must persist in the EVM account state.
    /// The invariant checks that the live balance of the target account
    /// matches the expected canonical value.
    #[test]
    fn deal_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            DealTarget::invariant_dealCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_deal must pass after setup"
        );
    }

    /// Re-dealing the expected balance in a later transaction must not
    /// corrupt the target account. This is the core property a stateful
    /// fuzzer relies on when actions need to restore canonical funding.
    #[test]
    fn restore_deal_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionRestoreDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::invariant_dealCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreDeal must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring deal"
        );
    }

    /// A single transaction can interleave multiple `vm.deal` calls and
    /// end on the expected balance without corrupting state. This proves
    /// the cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionDealSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::invariant_dealCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionDealSequence must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating balance and then restoring it in a sequence must leave the
    /// invariant intact. This mirrors how a stateful fuzzer would explore
    /// state mutations and then recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionMutateDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionRestoreDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::invariant_dealCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateDeal must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionRestoreDeal must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same balance when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_deal() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionRestoreDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::invariant_dealCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreDeal must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate account state
    /// by changing balance, and a final invariant verifies that the canonical
    /// value is still intact. This mirrors how a stateful fuzzer would use
    /// `vm.deal` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionRestoreDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionMutateDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionRestoreDealCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::actionDealSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                DealTarget::invariant_dealCall::new(()).abi_encode(),
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
