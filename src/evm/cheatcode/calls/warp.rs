//! `warp` cheatcode - set and persist `block.timestamp`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    value: U256,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.timestamp = value;
    ctx.set_block(block);
    state.block.timestamp = Some(value);
    Some(outcome::success())
}
#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::warp;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface WarpHarness {
            function setup() external;
            function getBlockTimestamp() external view returns (uint256);
            function actionWarp() external;
            function actionMutate() external;
            function invariant_warp() external view;
        }
    }

    const EXPECTED: U256 = U256::from_limbs([1_234_567_890, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/harness-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/WarpHarness.sol:WarpHarness");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // Harness-level unit tests

    /// vm.warp must set the EVM context block.timestamp and persist it in state.
    #[test]
    fn warp_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let outcome = warp::handle(&mut ctx, &mut state, EXPECTED);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.timestamp, EXPECTED,
            "ctx block.timestamp must be updated"
        );
        assert_eq!(
            state.block.timestamp,
            Some(EXPECTED),
            "state must record the value"
        );
    }

    /// vm.warp with U256::MAX must set the EVM context block.timestamp correctly.
    #[test]
    fn warp_large_number_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let outcome = warp::handle(&mut ctx, &mut state, U256::MAX);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.timestamp,
            U256::MAX,
            "ctx block.timestamp must be updated to U256::MAX"
        );
        assert_eq!(
            state.block.timestamp,
            Some(U256::MAX),
            "state must record the large value"
        );
    }

    // Integration tests

    /// `vm.warp` used during setup must persist into `chain.exec` so that a
    /// plain `block.timestamp` read in the first exec transaction returns the
    /// canonical value. This proves the block environment survives deployment
    /// and setup.
    #[test]
    fn setup_warp_persists_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            WarpHarness::getBlockTimestampCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "getBlockTimestamp must succeed after setup"
        );
        let value = WarpHarness::getBlockTimestampCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            value, EXPECTED,
            "block.timestamp must match the canonical value set during setup"
        );
    }

    /// Re-warping to the canonical value in a later transaction must not
    /// corrupt the expected timestamp. This is the core property a stateful
    /// fuzzer relies on when actions need to restore canonical block state.
    #[test]
    fn restore_warp_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::actionWarpCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::invariant_warpCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionWarp must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring timestamp"
        );
    }

    /// Warping to a non-canonical value in an action mutates observable block
    /// state, so the invariant must fail afterward. This proves the cheatcode
    /// actually changes the block environment.
    #[test]
    fn mutate_warp_breaks_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::actionMutateCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::invariant_warpCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionMutate must succeed");
        assert!(
            !execution.results[1].success,
            "invariant must fail after mutating timestamp"
        );
    }

    /// A cloned chain snapshot must produce the same timestamp when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_timestamp() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::actionWarpCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::invariant_warpCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionWarp must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// Cross-transaction determinism: re-warping in a second `exec` must
    /// still leave the canonical timestamp intact.
    #[test]
    fn deterministic_across_separate_execs() {
        let (mut chain, target) = deploy_and_setup();

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::actionWarpCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpHarness::invariant_warpCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "first exec must succeed"
        );

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "second exec must succeed"
        );
    }
}
