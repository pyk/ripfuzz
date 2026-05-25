//! `coinbase` cheatcode - set and persist `block.coinbase`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Address,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    state.block.beneficiary = Some(addr);
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
        interface CoinbaseTarget {
            function setup() external;
            function invariant_coinbase() external view;
            function actionRestoreCoinbase() external;
            function actionMutateCoinbase() external;
            function actionCoinbaseSequence() external;
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
        let contract = load_fixture("src/CoinbaseTarget.sol:CoinbaseTarget");
        let mut chain = Chain::empty(Config::default());
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.coinbase` used during setup must persist in EVM block state.
    /// The invariant checks that the live `block.coinbase` matches the
    /// expected canonical address.
    #[test]
    fn coinbase_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_coinbase must pass after setup"
        );
    }

    /// Re-setting the coinbase in a later transaction must not corrupt the
    /// expected value. This is the core property a stateful fuzzer relies
    /// on when actions need to restore canonical block producer state.
    #[test]
    fn restore_coinbase_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionRestoreCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreCoinbase must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring coinbase"
        );
    }

    /// A single transaction can interleave multiple `vm.coinbase` calls and
    /// end on the expected address without corrupting state. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionCoinbaseSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionCoinbaseSequence must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating coinbase and then restoring it in a sequence must leave the
    /// invariant intact. This mirrors how a stateful fuzzer would explore
    /// state mutations and then recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionMutateCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionRestoreCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateCoinbase must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionRestoreCoinbase must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same coinbase when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_coinbase() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionRestoreCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreCoinbase must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate block state by
    /// changing coinbase, and a final invariant verifies that the canonical
    /// address is still intact. This mirrors how a stateful fuzzer would
    /// use `vm.coinbase` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionRestoreCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionMutateCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionRestoreCoinbaseCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::actionCoinbaseSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode(),
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
