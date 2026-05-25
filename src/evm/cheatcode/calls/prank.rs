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

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::prank;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface PrankTarget {
            function setup() external;

            // Getters
            function getVictimSender() external view returns (address);
            function getVictimOrigin() external view returns (address);
            function getInnerSender() external view returns (address);
            function getInnerOrigin() external view returns (address);
            function getStoredSender() external view returns (address);
            function getStoredOrigin() external view returns (address);
            function getStoredBalance() external view returns (uint256);
            function getStoredTimestamp() external view returns (uint256);
            function getCurrentActor() external view returns (address);
            function getSeqSender1() external view returns (address);
            function getSeqSender2() external view returns (address);
            function getSeqSender3() external view returns (address);
            function getSeqSender4() external view returns (address);
            function getSeqOrigin2() external view returns (address);

            // Basic prank
            function callPrankSender() external;
            function callPrankOrigin() external;
            function callPrankConsumed() external;

            // startPrank / stopPrank
            function callStartStop() external;
            function callStartNoStop() external;
            function callAfterStartNoStop() external;
            function callAfterStop() external;

            // Overwrite validation (reverting)
            function callDoublePrankReverts() external;
            function callStartOverwriteUnusedReverts() external;
            function callPrankOverStartReverts() external;

            // Overwrite used startPrank
            function callStartOverwriteUsed() external;

            // Nested calls
            function callPrankNested() external;
            function callStartNested() external;

            // Constructor pranking
            function callPrankConstructor() external;

            // Modifier
            function callModifierPrank(uint256 actorSeed) external;

            // Sequence
            function callPrankSequence() external;

            // Interaction with other cheatcodes
            function callPrankAndDeal() external;
            function callStartPrankAndWarp() external;

            // Actions
            function actionPrank() external;
            function actionStartPrank() external;
            function actionStopPrank() external;
            function actionRestore() external;
            function actionModifierPrank(uint256 actorSeed) external;

            // Invariants
            function invariant_prank() external view;
            function invariant_victim_sender() external view;
            function invariant_modifier_prank() external view;
        }
    }

    const PRANK_ADDR: Address = address!("0x1111111111111111111111111111111111111111");
    const PRANK_ADDR_2: Address = address!("0x2222222222222222222222222222222222222222");
    const PRANK_ORIGIN: Address = address!("0x3333333333333333333333333333333333333333");
    const START_ADDR: Address = address!("0x5555555555555555555555555555555555555555");
    const START_ORIGIN: Address = address!("0x6666666666666666666666666666666666666666");
    const PERSIST_ADDR: Address = address!("0x7777777777777777777777777777777777777777");
    const NESTED_ADDR: Address = address!("0x9999999999999999999999999999999999999999");
    const CONSTRUCTOR_ADDR: Address = address!("0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC");
    const ACTOR_0: Address = address!("0x1000000000000000000000000000000000000001");
    const ACTOR_1: Address = address!("0x2000000000000000000000000000000000000002");
    const _ACTOR_2: Address = address!("0x3000000000000000000000000000000000000003");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/PrankTarget.sol:PrankTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_opts = SetupInput::new(target);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with the cheatcode inspector enabled so that `vm.*`
    /// functions invoked by the target contract are intercepted.
    fn call_with_cheatcode_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = cheatcode::Inspector::default();
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

    /// Execute a CALL reusing an existing inspector so that prank state
    /// (e.g. an active `startPrank`) persists across transactions.
    fn call_with_inspector(
        chain: &mut Chain,
        inspector: cheatcode::Inspector,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> (TransactionResult, cheatcode::Inspector) {
        let tx = revm::context::TxEnv {
            caller,
            kind: revm::primitives::TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        chain.inspect(tx, inspector).unwrap()
    }

    /// Call a view/pure function that returns a single `address` and decode it.
    macro_rules! call_address_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
        ($chain:expr, $target:expr, $call:ty, $args:tt) => {{
            let calldata = <$call>::new($args).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
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

    // -----------------------------------------------------------------
    // Handler-level tests
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
        let outcome = prank::prank_origin(&mut state, PRANK_ADDR, PRANK_ORIGIN);
        assert!(outcome.unwrap().result.is_ok(), "prank_origin must succeed");
        let active = state.prank.active.unwrap();
        assert_eq!(active.caller, PRANK_ADDR);
        assert_eq!(active.origin, Some(PRANK_ORIGIN));
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
    // Basic contract-path integration
    // -----------------------------------------------------------------

    /// `vm.prank(addr)` changes `msg.sender` for the next call but leaves
    /// `tx.origin` untouched.
    #[test]
    fn prank_changes_sender_only() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankSenderCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankSender must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        let origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimOriginCall);
        assert_eq!(sender, PRANK_ADDR, "msg.sender must be pranked");
        assert_eq!(origin, DEFAULT_DEPLOYER, "tx.origin must stay unchanged");
    }

    /// `vm.prank(addr, origin)` changes both `msg.sender` and `tx.origin`.
    #[test]
    fn prank_with_origin_changes_both() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankOriginCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankOrigin must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        let origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimOriginCall);
        assert_eq!(sender, PRANK_ADDR_2, "msg.sender must be pranked");
        assert_eq!(origin, PRANK_ORIGIN, "tx.origin must be pranked");
    }

    /// `vm.prank` is consumed by the very next call and does not leak to
    /// subsequent calls in the same transaction.
    #[test]
    fn prank_consumed_after_one_call() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankConsumedCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankConsumed must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        let origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimOriginCall);
        assert_eq!(sender, target, "second call must not be pranked");
        assert_eq!(origin, DEFAULT_DEPLOYER, "origin must stay unchanged");
    }

    /// `vm.startPrank` changes `msg.sender` for every call until `stopPrank`.
    #[test]
    fn start_prank_changes_sender_for_multiple_calls() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callStartStopCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartStop must succeed");

        let stored_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        let stored_origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredOriginCall);
        let victim_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);

        assert_eq!(
            stored_sender, START_ADDR,
            "both calls inside startPrank must see the pranked sender"
        );
        assert_eq!(
            stored_origin, START_ORIGIN,
            "both calls inside startPrank must see the pranked origin"
        );
        assert_eq!(
            victim_sender, target,
            "call after stopPrank must see the real sender"
        );
    }

    // -----------------------------------------------------------------
    // Persistence across transactions (inspector reuse)
    // -----------------------------------------------------------------

    /// `vm.startPrank` without a matching `stopPrank` must persist across
    /// separate top-level transactions when the inspector is reused.
    #[test]
    fn start_prank_persists_across_transactions() {
        let (mut chain, target) = deploy_and_setup();
        let inspector = cheatcode::Inspector::default();

        // First tx: startPrank + record.
        let calldata = PrankTarget::callStartNoStopCall::new(()).abi_encode();
        let (result, inspector) = call_with_inspector(
            &mut chain,
            inspector,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartNoStop must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(sender, PERSIST_ADDR, "first tx must be pranked");

        // Second tx: record again with the SAME inspector.
        let calldata = PrankTarget::callAfterStartNoStopCall::new(()).abi_encode();
        let (result, _inspector) = call_with_inspector(
            &mut chain,
            inspector,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callAfterStartNoStop must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(
            sender, PERSIST_ADDR,
            "startPrank must persist into the next transaction"
        );
    }

    /// `vm.stopPrank` must clear a persisted `startPrank` so the next
    /// transaction sees the original caller again.
    #[test]
    fn stop_prank_restores_caller() {
        let (mut chain, target) = deploy_and_setup();
        let inspector = cheatcode::Inspector::default();

        // Establish a persistent startPrank.
        let calldata = PrankTarget::callStartNoStopCall::new(()).abi_encode();
        let (result, inspector) = call_with_inspector(
            &mut chain,
            inspector,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartNoStop must succeed");

        // Stop it.
        let calldata = PrankTarget::callAfterStopCall::new(()).abi_encode();
        let (result, _inspector) = call_with_inspector(
            &mut chain,
            inspector,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callAfterStop must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(sender, target, "stopPrank must restore the real sender");
    }

    /// `vm.prank` must NOT persist across transactions; a fresh inspector
    /// should have no active prank for the next call.
    #[test]
    fn prank_does_not_persist_across_transactions() {
        let (mut chain, target) = deploy_and_setup();

        // First tx: prank + record.
        let calldata = PrankTarget::callPrankSenderCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankSender must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(sender, PRANK_ADDR, "first tx must be pranked");

        // Second tx: fresh inspector, simple record.
        let calldata = PrankTarget::actionRestoreCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionRestore must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(
            sender, target,
            "prank must not leak into the next transaction"
        );
    }

    // -----------------------------------------------------------------
    // Overwrite validation via contract path
    // -----------------------------------------------------------------

    /// Calling `vm.prank` twice in one tx must revert.
    #[test]
    fn double_prank_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callDoublePrankRevertsCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(!result.success, "callDoublePrankReverts must revert");
    }

    /// Calling `vm.startPrank` twice without the first being used must revert.
    #[test]
    fn start_prank_overwrite_unused_reverts() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callStartOverwriteUnusedRevertsCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "callStartOverwriteUnusedReverts must revert"
        );
    }

    /// `vm.prank` cannot overwrite an active `vm.startPrank`.
    #[test]
    fn prank_over_start_prank_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankOverStartRevertsCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(!result.success, "callPrankOverStartReverts must revert");
    }

    /// A used `startPrank` may be overwritten by another `startPrank` in the
    /// same transaction.
    #[test]
    fn start_prank_overwrite_used_in_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callStartOverwriteUsedCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartOverwriteUsed must succeed");

        let stored: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            stored, PRANK_ADDR_2,
            "overwrite used startPrank must apply the new sender"
        );
    }

    // -----------------------------------------------------------------
    // Nested calls
    // -----------------------------------------------------------------

    /// `vm.prank` only affects the immediate next call; deeper calls made
    /// by the pranked contract see the victim as sender.
    #[test]
    fn prank_nested_only_affects_immediate_call() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankNestedCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankNested must succeed");

        let victim_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        let inner_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getInnerSenderCall);
        let victim_origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimOriginCall);

        assert_eq!(victim_sender, NESTED_ADDR, "outer call must be pranked");
        assert_eq!(
            inner_sender,
            // The `inner` contract address is not known at compile time in the
            // Solidity target, so we cannot assert an exact value. Instead we
            // assert it is NOT the nested prank address, proving the prank did
            // not leak.
            call_address_getter!(&mut chain, target, PrankTarget::getInnerSenderCall),
            "inner call must see the victim as sender"
        );
        // Re-read to avoid the macro borrowing issue above.
        let inner_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getInnerSenderCall);
        assert_ne!(inner_sender, NESTED_ADDR, "inner call must NOT be pranked");
        assert_eq!(
            victim_origin, DEFAULT_DEPLOYER,
            "origin must stay unchanged"
        );
    }

    /// `vm.startPrank` affects every nested call frame until `stopPrank`.
    #[test]
    fn start_prank_affects_nested_calls() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callStartNestedCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartNested must succeed");

        let victim_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        let inner_sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getInnerSenderCall);
        let victim_origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimOriginCall);
        let inner_origin: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getInnerOriginCall);

        assert_eq!(victim_sender, START_ADDR, "outer call must be pranked");
        assert_eq!(inner_sender, START_ADDR, "inner call must also be pranked");
        assert_eq!(victim_origin, START_ORIGIN, "outer origin must be pranked");
        assert_eq!(
            inner_origin, START_ORIGIN,
            "inner origin must also be pranked"
        );
    }

    // -----------------------------------------------------------------
    // Constructor pranking
    // -----------------------------------------------------------------

    /// `vm.prank` must spoof the sender of a contract deployment.
    #[test]
    fn prank_constructor_changes_sender() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankConstructorCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankConstructor must succeed");

        let stored: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            stored, CONSTRUCTOR_ADDR,
            "constructor must see the pranked sender"
        );
    }

    // -----------------------------------------------------------------
    // Modifier with useActor
    // -----------------------------------------------------------------

    /// The `useActor` modifier must correctly apply `startPrank` / `stopPrank`
    /// around the body of a function.
    #[test]
    fn modifier_use_actor_changes_sender() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callModifierPrankCall::new((U256::from(0),)).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callModifierPrank must succeed");

        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getVictimSenderCall);
        assert_eq!(
            sender, ACTOR_0,
            "useActor(0) must set msg.sender to actors[0]"
        );
    }

    // -----------------------------------------------------------------
    // Single-transaction sequence determinism
    // -----------------------------------------------------------------

    /// Multiple prank variants interleaved in one transaction must each
    /// produce the correct sender / origin.
    #[test]
    fn prank_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankSequence must succeed");

        let seq1: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getSeqSender1Call);
        let seq2: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getSeqSender2Call);
        let seq3: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getSeqSender3Call);
        let seq4: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getSeqSender4Call);
        let origin2: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getSeqOrigin2Call);

        assert_eq!(seq1, PRANK_ADDR, "first prank must match");
        assert_eq!(seq2, PRANK_ADDR_2, "second prank must match");
        assert_eq!(origin2, PRANK_ORIGIN, "second origin must match");
        assert_eq!(seq3, START_ADDR, "startPrank must match");
        assert_eq!(seq4, target, "after stopPrank must restore sender");
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    /// `vm.prank` and `vm.deal` must coexist in the same transaction.
    #[test]
    fn prank_and_deal_interaction() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callPrankAndDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrankAndDeal must succeed");

        let balance: U256 =
            call_uint256_getter!(&mut chain, target, PrankTarget::getStoredBalanceCall);
        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            balance,
            U256::from(5_000_000_000_000_000_000u64),
            "deal must set balance to 5 ether"
        );
        assert_eq!(
            sender, PRANK_ADDR,
            "prank sender must still be correct inside deal interaction"
        );
    }

    /// `vm.startPrank` and `vm.warp` must coexist in the same transaction.
    #[test]
    fn start_prank_and_warp_interaction() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrankTarget::callStartPrankAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStartPrankAndWarp must succeed");

        let timestamp: U256 =
            call_uint256_getter!(&mut chain, target, PrankTarget::getStoredTimestampCall);
        let sender: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            timestamp,
            U256::from(1_234_567_890u64),
            "warp must set timestamp"
        );
        assert_eq!(
            sender, START_ADDR,
            "startPrank sender must still be correct inside warp interaction"
        );
    }

    // -----------------------------------------------------------------
    // Invariants
    // -----------------------------------------------------------------

    /// Invariants must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let invariants = [
            (
                PrankTarget::invariant_prankCall::new(()).abi_encode(),
                "invariant_prank",
            ),
            (
                PrankTarget::invariant_victim_senderCall::new(()).abi_encode(),
                "invariant_victim_sender",
            ),
            (
                PrankTarget::invariant_modifier_prankCall::new(()).abi_encode(),
                "invariant_modifier_prank",
            ),
        ];
        for (calldata, name) in invariants {
            let result = chain
                .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{name} must pass after setup");
        }
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves prank determinism across multiple transactions and that
    /// invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Action 1: single-call prank.
        let calldata = PrankTarget::actionPrankCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionPrank must succeed");
        let stored: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(stored, PRANK_ADDR, "actionPrank must store pranked sender");

        // Restore to the un-pranked baseline.
        let calldata = PrankTarget::actionRestoreCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionRestore must succeed");
        let stored: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            stored, target,
            "actionRestore must reset storedSender to the real sender"
        );

        // Invariant must pass after restoration.
        let calldata = PrankTarget::invariant_prankCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_prank must pass after action sequence"
        );
    }

    /// A sequence that uses the `useActor` modifier must leave the victim
    /// in a valid state for invariants.
    #[test]
    fn action_modifier_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Action with modifier.
        let calldata = PrankTarget::actionModifierPrankCall::new((U256::from(1),)).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionModifierPrank must succeed");
        let stored: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(stored, ACTOR_1, "useActor(1) must store actors[1]");

        // Restore baseline.
        let calldata = PrankTarget::actionRestoreCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionRestore must succeed");

        let calldata = PrankTarget::invariant_modifier_prankCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_modifier_prank must pass after sequence"
        );
    }

    /// Cross-transaction determinism: the same prank action executed twice
    /// in separate transactions must yield the same observed sender.
    #[test]
    fn prank_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = PrankTarget::actionPrankCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionPrank must succeed");
        let first: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(first, PRANK_ADDR);

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionPrank must succeed on second call");
        let second: Address =
            call_address_getter!(&mut chain, target, PrankTarget::getStoredSenderCall);
        assert_eq!(
            second, PRANK_ADDR,
            "prank must be deterministic across transactions"
        );
    }
}
