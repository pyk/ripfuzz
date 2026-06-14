//! `addr` cheatcode - derive an address from a private key.

use alloy_primitives::{Address, U256};
use k256::elliptic_curve::{Curve, bigint::ArrayEncoding};
use revm::interpreter::CallOutcome;

use crate::evm::cheatcode::outcome;

pub fn handle(sk: U256) -> Option<CallOutcome> {
    if sk.is_zero() {
        return Some(outcome::revert("private key cannot be 0"));
    }
    let order = U256::from_be_slice(&k256::Secp256k1::ORDER.to_be_byte_array());
    if sk >= order {
        return Some(outcome::revert(&format!(
            "private key must be less than the secp256k1 curve order ({order})"
        )));
    }
    let sk_bytes = sk.to_be_bytes_vec();
    let signing_key = k256::ecdsa::SigningKey::from_slice(&sk_bytes).ok()?;
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_encoded_point(false);
    let pk_bytes = public_key.as_bytes();
    if pk_bytes.len() != 65 {
        return Some(outcome::revert("invalid public key length"));
    }
    let hash = alloy_primitives::keccak256(&pk_bytes[1..]);
    let address = Address::from_slice(&hash[12..]);
    Some(outcome::success_address(address))
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
        interface AddrHandler {
            function setup() external;
            function invariant_actorsMatch() external view;
            function actionRefreshAdmin() external;
            function actionRefreshVoter() external;
            function actionRefreshProposer() external;
            function actionRefreshAll() external;
            function actionRefreshInterleaved() external;
            function actionInvalidZero() external pure;
            function actionInvalidOrder() external pure;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/handler-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/AddrHandler.sol:AddrHandler");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.addr` used during setup must store the well-known addresses.
    /// The invariant checks that all three actors match their expected identities.
    #[test]
    fn actors_derived_in_setup_match_well_known() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_actorsMatch must pass after setup"
        );
    }

    /// Re-deriving an address in a later transaction and overwriting storage
    /// must not change the actor identity. This is the core property a
    /// stateful fuzzer relies on when actions need known signer addresses.
    #[test]
    fn recompute_and_refresh_in_action_preserves_identity() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRefreshAdmin must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after refreshing admin"
        );
    }

    /// A single transaction can re-derive multiple actor addresses without
    /// corrupting results. This proves `vm.addr` is stateless and safe to
    /// call repeatedly inside one tx.
    #[test]
    fn batch_refresh_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRefreshAll must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after batch refresh"
        );
    }

    /// Interleaving different private keys in the same transaction must not
    /// cause cross-key pollution. Each key must still map to its correct
    /// address.
    #[test]
    fn interleaved_keys_produce_consistent_results() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshInterleavedCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRefreshInterleaved must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after interleaved refresh"
        );
    }

    /// `vm.addr(0)` must revert because 0 is not a valid private key.
    #[test]
    fn invalid_key_zero_reverts() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            AddrHandler::actionInvalidZeroCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.addr(0) must revert in a transaction"
        );
    }

    /// `vm.addr` with a key >= secp256k1 curve order must revert.
    #[test]
    fn invalid_key_order_reverts() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            AddrHandler::actionInvalidOrderCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.addr(order) must revert in a transaction"
        );
    }

    /// A cloned chain snapshot must produce the same addresses when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_produces_same_addresses() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
            )),
        ];
        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRefreshAll must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate storage by
    /// refreshing different actors, and a final invariant verifies that
    /// all identities are still intact. This mirrors how a stateful fuzzer
    /// would use `vm.addr` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshAdminCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshVoterCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshProposerCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::actionRefreshAllCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                AddrHandler::invariant_actorsMatchCall::new(()).abi_encode(),
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
