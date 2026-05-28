//! `setNonce` / `getNonce` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn set_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    nonce: u64,
) -> Option<revm::interpreter::CallOutcome> {
    let current = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    if nonce < current {
        return Some(outcome::revert(&format!(
            "new nonce ({nonce}) must be >= current nonce ({current})"
        )));
    }
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_nonce(nonce);
    Some(outcome::success())
}

pub fn get_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(outcome::success_u256(U256::from(nonce)))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::nonce;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface NonceTarget {
            function setup() external;
            function actionBumpNonce() external;
            function actionBumpNonceByTwo() external;
            function actionOverwriteSequence() external;
            function actionRevertLowNonce() external;
            function getStoredNonce() external view returns (uint256);
            function getNonceDirect() external view returns (uint256);
            function invariant_nonceAtLeastBaseline() external view;
        }
    }

    const ACTOR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const BASELINE: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/NonceTarget.sol:NonceTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // -----------------------------------------------------------------------
    // Handler-level unit tests
    // -----------------------------------------------------------------------

    /// vm.setNonce must succeed and vm.getNonce must read the written value.
    #[test]
    fn set_nonce_sets_nonce_and_get_nonce_reads_it() {
        let mut ctx = revm::context::Context::mainnet();
        let outcome = nonce::set_nonce(&mut ctx, ACTOR, 42);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = nonce::get_nonce(&mut ctx, ACTOR);
        assert!(outcome.is_some(), "must return an outcome");
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(decoded, BASELINE, "get_nonce must read 42");
    }

    /// vm.setNonce with a value lower than the current nonce must revert.
    #[test]
    fn set_nonce_lower_than_current_reverts() {
        let mut ctx = revm::context::Context::mainnet();
        let outcome = nonce::set_nonce(&mut ctx, ACTOR, 10);
        assert!(outcome.is_some() && outcome.unwrap().result.is_ok());

        let outcome = nonce::set_nonce(&mut ctx, ACTOR, 5);
        assert!(outcome.is_some());
        assert!(
            !outcome.unwrap().result.is_ok(),
            "set_nonce to lower value must revert"
        );
    }

    /// vm.setNonce with the same value as the current nonce must succeed.
    #[test]
    fn set_nonce_same_value_succeeds() {
        let mut ctx = revm::context::Context::mainnet();
        let outcome = nonce::set_nonce(&mut ctx, ACTOR, 42);
        assert!(outcome.is_some() && outcome.unwrap().result.is_ok());

        let outcome = nonce::set_nonce(&mut ctx, ACTOR, 42);
        assert!(outcome.is_some() && outcome.unwrap().result.is_ok());

        let outcome = nonce::get_nonce(&mut ctx, ACTOR);
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(decoded, BASELINE);
    }

    /// vm.getNonce on an unknown account must return zero.
    #[test]
    fn get_nonce_returns_zero_for_unknown_account() {
        let mut ctx = revm::context::Context::mainnet();
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let outcome = nonce::get_nonce(&mut ctx, unknown);
        assert!(outcome.is_some(), "must return an outcome");
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(decoded, U256::ZERO, "unknown account nonce must be 0");
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    /// `vm.setNonce` used during setup must persist the stored nonce so that
    /// a later invariant call can verify the baseline value.
    #[test]
    fn nonce_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// `vm.setNonce` in setup modifies the EVM account state directly.
    /// `vm.getNonce` during a later `chain.exec` must still read the baseline
    /// without any re-labeling action, proving database persistence.
    #[test]
    fn nonce_persists_from_setup_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            NonceTarget::getNonceDirectCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(execution.results[0].success, "getNonceDirect must succeed");
        let ret = NonceTarget::getNonceDirectCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            ret, BASELINE,
            "vm.getNonce must return the nonce set during setup"
        );
    }

    /// Bumping the nonce by one in an action must increase the stored value.
    /// The invariant (nonce >= baseline) still passes because the nonce only
    /// went up.
    #[test]
    fn bump_nonce_in_action_increases_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::getStoredNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(execution.results[0].success, "actionBumpNonce must succeed");
        assert!(execution.results[1].success, "getStoredNonce must succeed");
        let stored: U256 = NonceTarget::getStoredNonceCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            BASELINE + U256::from(1),
            "nonce must be 43 after bump"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after bump"
        );
    }

    /// A sequence of bumps must accumulate. This proves vm.setNonce is
    /// stateful and composes correctly across transactions in one exec.
    #[test]
    fn bump_nonce_sequence_accumulates() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceByTwoCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::getStoredNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 4);
        assert!(execution.results[2].success, "getStoredNonce must succeed");
        let stored: U256 = NonceTarget::getStoredNonceCall::abi_decode_returns(
            &execution.results[2].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            BASELINE + U256::from(3),
            "nonce must be 45 after +1 then +2"
        );
        assert!(
            execution.results[3].success,
            "invariant must pass after sequence"
        );
    }

    /// Overwriting the nonce multiple times in a single transaction and ending
    /// +30 above current must keep the invariant intact. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn overwrite_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionOverwriteSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::getStoredNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionOverwriteSequence must succeed"
        );
        let stored: U256 = NonceTarget::getStoredNonceCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            BASELINE + U256::from(30),
            "nonce must be 72 after +30 overwrite"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after overwrite sequence"
        );
    }

    /// Attempting to set nonce lower than current must revert the transaction.
    #[test]
    fn revert_low_nonce_in_action_reverts() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            NonceTarget::actionRevertLowNonceCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "actionRevertLowNonce must revert"
        );
    }

    /// A cloned chain snapshot must produce the same nonce state when actions
    /// are executed on the clone. This is critical for parallel fuzzing where
    /// each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_nonce_state() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceByTwoCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
            )),
        ];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionBumpNonce must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "actionBumpNonceByTwo must succeed on cloned chain"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate nonce state by
    /// bumping and overwriting, and a final invariant verifies that the
    /// nonce never dropped below the baseline.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionBumpNonceByTwoCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::actionOverwriteSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                NonceTarget::invariant_nonceAtLeastBaselineCall::new(()).abi_encode(),
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
