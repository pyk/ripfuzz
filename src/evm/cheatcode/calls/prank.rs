//! Prank cheatcodes - `vm.prank`, `vm.startPrank`, `vm.stopPrank`.

use revm::primitives::Address;

use crate::evm::cheatcode::{
    outcome,
    state::{ExecutionState, PrankState, StartPrankState},
};

pub fn prank(state: &mut ExecutionState, addr: Address) -> Option<revm::interpreter::CallOutcome> {
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "prank(address) cannot be called when a prank is already active",
        ));
    }
    if state.prank.start.is_some() {
        return Some(outcome::revert(
            "prank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.active = Some(PrankState {
        caller: addr,
        origin: None,
        prank_caller: Address::ZERO,
        set_depth: 0,
        single_call: true,
        used: false,
    });
    Some(outcome::success())
}

pub fn prank_origin(
    state: &mut ExecutionState,
    addr: Address,
    origin: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "prank(address) cannot be called when a prank is already active",
        ));
    }
    if state.prank.start.is_some() {
        return Some(outcome::revert(
            "prank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.active = Some(PrankState {
        caller: addr,
        origin,
        prank_caller: Address::ZERO,
        set_depth: 0,
        single_call: true,
        used: false,
    });
    Some(outcome::success())
}

pub fn start_prank(
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a prank is already active",
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin: None,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(outcome::success())
}

pub fn start_prank_origin(
    state: &mut ExecutionState,
    addr: Address,
    origin: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a prank is already active",
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(outcome::success())
}

pub fn stop_prank(state: &mut ExecutionState) -> Option<revm::interpreter::CallOutcome> {
    state.prank.active = None;
    state.prank.start = None;
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::prank;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface PrankTarget {
            function setup() external;
            function actionNestedCall() external;
            function actionOverwriteStart() external;
            function actionStopPrank() external;
            function actionRestoreAdmin() external;
            function actionUseActor(uint256 actorSeed) external;
            function actionRevertDoublePrank() external;
            function actionRevertDoubleStart() external;
            function actionRevertPrankOverStart() external;
            function getLastSender() external view returns (address);
            function invariant_senderIsAdmin() external view;
            function invariant_senderIsUser() external view;
            function invariant_senderIsTarget() external view;
            function invariant_senderValid() external view;
        }
    }

    alloy_sol_types::sol! {
        interface PrankLeakTarget {
            function setup() external;
            function action() external;
            function invariant() external view;
        }
    }

    const PRANK_ADDR: Address = address!("0x1111111111111111111111111111111111111111");
    const PRANK_ADDR_2: Address = address!("0x2222222222222222222222222222222222222222");
    const START_ADDR: Address = address!("0x5555555555555555555555555555555555555555");
    const START_ORIGIN: Address = address!("0x6666666666666666666666666666666666666666");
    const ACTOR_1: Address = address!("0x2000000000000000000000000000000000000002");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/PrankTarget.sol:PrankTarget");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    fn deploy_and_setup_leak() -> (Chain, Address) {
        let contract = load_fixture("src/PrankLeakTarget.sol:PrankLeakTarget");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // -----------------------------------------------------------------
    // Handler-level unit tests
    // -----------------------------------------------------------------

    /// `prank::prank` must register an active single-call prank.
    #[test]
    fn prank_sets_active_state() {
        let mut state = ExecutionState::default();
        let outcome = prank::prank(&mut state, PRANK_ADDR);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(outcome.unwrap().result.is_ok(), "prank must succeed");
        assert_eq!(state.prank.active.unwrap().caller, PRANK_ADDR);
        assert!(state.prank.start.is_none());
    }

    /// `prank::prank_origin` must register an active prank with a custom origin.
    #[test]
    fn prank_origin_sets_both() {
        let mut state = ExecutionState::default();
        let outcome = prank::prank_origin(&mut state, PRANK_ADDR, START_ORIGIN);
        assert!(outcome.unwrap().result.is_ok(), "prank_origin must succeed");
        let active = state.prank.active.unwrap();
        assert_eq!(active.caller, PRANK_ADDR);
        assert_eq!(active.origin, Some(START_ORIGIN));
    }

    /// `prank::start_prank` must register a persistent start-prank.
    #[test]
    fn start_prank_sets_start_state() {
        let mut state = ExecutionState::default();
        let outcome = prank::start_prank(&mut state, START_ADDR);
        assert!(outcome.unwrap().result.is_ok(), "start_prank must succeed");
        assert_eq!(state.prank.start.unwrap().caller, START_ADDR);
        assert!(state.prank.active.is_none());
    }

    /// `prank::start_prank_origin` must register a persistent start-prank with origin.
    #[test]
    fn start_prank_origin_sets_both() {
        let mut state = ExecutionState::default();
        let outcome = prank::start_prank_origin(&mut state, START_ADDR, START_ORIGIN);
        assert!(
            outcome.unwrap().result.is_ok(),
            "start_prank_origin must succeed"
        );
        let start = state.prank.start.unwrap();
        assert_eq!(start.caller, START_ADDR);
        assert_eq!(start.origin, Some(START_ORIGIN));
    }

    /// `prank::stop_prank` must clear both active and start prank state.
    #[test]
    fn stop_prank_clears_both() {
        let mut state = ExecutionState::default();
        prank::prank(&mut state, PRANK_ADDR);
        prank::start_prank(&mut state, START_ADDR);
        let outcome = prank::stop_prank(&mut state);
        assert!(outcome.unwrap().result.is_ok(), "stop_prank must succeed");
        assert!(state.prank.active.is_none());
        assert!(state.prank.start.is_none());
    }

    /// Calling `prank` twice without the first being consumed must revert.
    #[test]
    fn double_prank_reverts() {
        let mut state = ExecutionState::default();
        prank::prank(&mut state, PRANK_ADDR);
        let outcome = prank::prank(&mut state, PRANK_ADDR_2);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(!outcome.unwrap().result.is_ok(), "double prank must revert");
    }

    /// Calling `start_prank` twice without the first being used must revert.
    #[test]
    fn double_start_prank_reverts() {
        let mut state = ExecutionState::default();
        prank::start_prank(&mut state, PRANK_ADDR);
        let outcome = prank::start_prank(&mut state, PRANK_ADDR_2);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(
            !outcome.unwrap().result.is_ok(),
            "double startPrank must revert"
        );
    }

    /// `prank` cannot be called while a `startPrank` is active.
    #[test]
    fn prank_over_start_prank_reverts() {
        let mut state = ExecutionState::default();
        prank::start_prank(&mut state, PRANK_ADDR);
        let outcome = prank::prank(&mut state, PRANK_ADDR_2);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(
            !outcome.unwrap().result.is_ok(),
            "prank over startPrank must revert"
        );
    }

    /// A used `startPrank` may be overwritten by another `startPrank`.
    #[test]
    fn start_prank_overwrite_used_succeeds() {
        let mut state = ExecutionState::default();
        prank::start_prank(&mut state, PRANK_ADDR);
        state.prank.start.as_mut().unwrap().used = true;
        let outcome = prank::start_prank(&mut state, PRANK_ADDR_2);
        assert!(
            outcome.unwrap().result.is_ok(),
            "overwrite used startPrank must succeed"
        );
        assert_eq!(state.prank.start.unwrap().caller, PRANK_ADDR_2);
    }

    // -----------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------

    /// `vm.startPrank(ADMIN)` used during setup must persist into `chain.exec`
    /// so that a nested call without any additional prank cheatcode still sees
    /// ADMIN as `msg.sender`. This is the baseline for stateful fuzzing.
    #[test]
    fn setup_persists_start_prank_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            PrankTarget::invariant_senderIsAdminCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// A nested call made without any prank cheatcode in the action must
    /// still see the persisted admin sender, proving the startPrank set
    /// during setup is active across the whole exec.
    #[test]
    fn persistent_prank_applies_to_nested_calls() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionNestedCallCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsAdminCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionNestedCall must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after nested call"
        );
    }

    /// Overwriting the used startPrank with `vm.startPrank(USER)` must change
    /// the sender for subsequent calls, proving a used startPrank can be
    /// replaced safely.
    #[test]
    fn overwrite_used_start_prank() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionOverwriteStartCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsUserCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionOverwriteStart must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must show USER after overwrite"
        );
    }

    /// `vm.stopPrank()` must clear the persistent prank so that a subsequent
    /// nested call sees the real caller (this contract) instead of the admin.
    #[test]
    fn stop_prank_restores_real_sender() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionStopPrankCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsTargetCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionStopPrank must succeed");
        assert!(
            execution.results[1].success,
            "invariant must show target address after stopPrank"
        );
    }

    /// Stopping the current prank and restoring the canonical admin prank
    /// in a single sequence must leave the invariant intact. This mirrors
    /// how a fuzzer would recover canonical state after an exploratory stop.
    #[test]
    fn stop_and_restore_cycle() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionStopPrankCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsTargetCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionRestoreAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsAdminCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(execution.results[0].success, "actionStopPrank must succeed");
        assert!(
            execution.results[1].success,
            "invariant must show target after stop"
        );
        assert!(
            execution.results[2].success,
            "actionRestoreAdmin must succeed"
        );
        assert!(
            execution.results[3].success,
            "invariant must show admin after restore"
        );
    }

    /// The `useActor` modifier must prank the chosen actor for the
    /// duration of the action and then clean up via `vm.stopPrank` so
    /// that subsequent calls see the real caller again.
    #[test]
    fn use_actor_modifier_pranks_and_cleans_up() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionUseActorCall::new((U256::from(1),)).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::getLastSenderCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionNestedCallCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsTargetCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(execution.results[0].success, "actionUseActor must succeed");
        assert!(execution.results[1].success, "getLastSender must succeed");
        let sender = PrankTarget::getLastSenderCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(sender, ACTOR_1, "modifier must set msg.sender to actors[1]");
        assert!(
            execution.results[2].success,
            "actionNestedCall must succeed"
        );
        assert!(
            execution.results[3].success,
            "invariant must show target after modifier cleanup"
        );
    }

    /// Invalid prank configurations must revert when executed through
    /// `chain.exec`. Each invalid case is tested in its own transaction.
    #[test]
    fn invalid_prank_configs_revert() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionRevertDoublePrankCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionRevertDoubleStartCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionRevertPrankOverStartCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(!execution.results[0].success, "double prank must revert");
        assert!(
            !execution.results[1].success,
            "double startPrank must revert"
        );
        assert!(
            !execution.results[2].success,
            "prank over startPrank must revert"
        );
    }

    /// A cloned chain snapshot must produce the same prank state when
    /// actions are executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_prank_state() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::actionNestedCallCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankTarget::invariant_senderIsAdminCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionNestedCall must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// Regression test: vm.startPrank must not leak into sub-calls made by
    /// contracts that were called with the pranked address.
    #[test]
    fn start_prank_does_not_leak_to_sub_calls() {
        let (mut chain, target) = deploy_and_setup_leak();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrankLeakTarget::actionCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrankLeakTarget::invariantCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "action must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass: startPrank must not leak into sub-calls"
        );
    }
}
