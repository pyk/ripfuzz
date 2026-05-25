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
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use k256::elliptic_curve::{Curve, bigint::ArrayEncoding};
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::addr;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface AddrTarget {
            function addrFromOne() external view returns (address);
            function addrFromTwo() external view returns (address);
            function getAddrFromMaxValid() external view returns (address);
            function addrFromZero() external pure;
            function addrFromOrder() external pure;
            function callAddrSameKeyTwice() external pure returns (address a, address b);
            function callAddrSequence() external pure returns (address first, address second, address third);
            function setup() external;

            // Fuzzing actions
            function actionAddrOne() external;
            function actionAddrTwo() external;
            function actionAddrMaxValid() external;

            // Invariants
            function invariant_addr_from_one() external view;
            function invariant_addr_from_two() external view;
            function invariant_addr_from_max_valid() external view;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/AddrTarget.sol:AddrTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(AddrTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupInput::new(target, setup_data);
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

    /// Call a view/pure function that returns a single address and decode it.
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
    }

    /// addr(1) must derive the well-known test address.
    #[test]
    fn addr_from_one_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let decoded = call_address_getter!(&mut chain, target, AddrTarget::addrFromOneCall);
        let expected = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        assert_eq!(
            decoded, expected,
            "vm.addr(1) must match the well-known address"
        );
    }

    /// addr(2) must derive the second well-known test address.
    #[test]
    fn addr_from_two_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let decoded = call_address_getter!(&mut chain, target, AddrTarget::addrFromTwoCall);
        let expected = address!("0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF");
        assert_eq!(
            decoded, expected,
            "vm.addr(2) must match the well-known address"
        );
    }

    /// addr(MAX_VALID_KEY) must succeed and return a valid address.
    #[test]
    fn addr_from_max_valid_key_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let decoded = call_address_getter!(&mut chain, target, AddrTarget::getAddrFromMaxValidCall);
        let expected = address!("0x80C0dbf239224071c59dD8970ab9d542E3414aB2");
        assert_eq!(
            decoded, expected,
            "vm.addr(secp256k1_order - 1) must match the well-known address"
        );
    }

    /// Calling addrFromZero() must revert because vm.addr(0) reverts.
    #[test]
    fn addr_zero_key_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = AddrTarget::addrFromZeroCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "addrFromZero() must revert when vm.addr(0) is called"
        );
    }

    /// Calling addrFromOrder() must revert because vm.addr(order) reverts.
    #[test]
    fn addr_order_key_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = AddrTarget::addrFromOrderCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "addrFromOrder() must revert when vm.addr(order) is called"
        );
    }

    /// vm.addr(1) called twice in the same transaction must return the same
    /// address, proving the cheatcode is deterministic and stateless.
    #[test]
    fn addr_same_key_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = AddrTarget::callAddrSameKeyTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callAddrSameKeyTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = AddrTarget::callAddrSameKeyTwiceCall::abi_decode_returns(&output).unwrap();
        let (a, b) = (ret.a, ret.b);
        assert_eq!(a, b, "vm.addr(1) called twice in one tx must be identical");
        let expected = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        assert_eq!(a, expected, "both calls must match the well-known address");
    }

    /// vm.addr must return the same address for the same key even when
    /// interleaved with calls for different keys, and different keys must
    /// produce different addresses.
    #[test]
    fn addr_sequence_returns_consistent_and_unique_addresses() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = AddrTarget::callAddrSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callAddrSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = AddrTarget::callAddrSequenceCall::abi_decode_returns(&output).unwrap();
        let (first, second, third) = (ret.first, ret.second, ret.third);

        assert_eq!(
            first, third,
            "vm.addr(1) must give the same address when interleaved with vm.addr(2)"
        );
        assert_ne!(
            first, second,
            "vm.addr(1) and vm.addr(2) must produce different addresses"
        );

        let expected_one = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        let expected_two = address!("0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF");
        assert_eq!(first, expected_one);
        assert_eq!(second, expected_two);
    }

    /// The address derived during setup must still be returned by the getter
    /// in a later transaction, proving contract-level persistence works.
    #[test]
    fn addr_setup_value_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();

        // Call the getter multiple times; each must return the same stored value.
        let first = call_address_getter!(&mut chain, target, AddrTarget::addrFromOneCall);
        let second = call_address_getter!(&mut chain, target, AddrTarget::addrFromOneCall);
        let expected = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
        assert_eq!(first, expected);
        assert_eq!(second, expected);
        assert_eq!(
            first, second,
            "getter must return the same stored address across calls"
        );
    }

    /// Invariants must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let invariants = [
            (
                AddrTarget::invariant_addr_from_oneCall::new(()).abi_encode(),
                "invariant_addr_from_one",
            ),
            (
                AddrTarget::invariant_addr_from_twoCall::new(()).abi_encode(),
                "invariant_addr_from_two",
            ),
            (
                AddrTarget::invariant_addr_from_max_validCall::new(()).abi_encode(),
                "invariant_addr_from_max_valid",
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
    /// This proves `vm.addr` stays deterministic across multiple transactions
    /// and that invariants correctly observe the persisted state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Action 1: re-derive addr(1) and store it.
        let calldata = AddrTarget::actionAddrOneCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionAddrOne must succeed");

        // Invariant 1 must still pass after the action.
        let calldata = AddrTarget::invariant_addr_from_oneCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_addr_from_one must pass after actionAddrOne"
        );

        // Action 2: re-derive addr(2) and store it.
        let calldata = AddrTarget::actionAddrTwoCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionAddrTwo must succeed");

        // Invariant 2 must still pass.
        let calldata = AddrTarget::invariant_addr_from_twoCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_addr_from_two must pass after actionAddrTwo"
        );

        // Action 3: re-derive addr(max) and store it.
        let calldata = AddrTarget::actionAddrMaxValidCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionAddrMaxValid must succeed");

        // Invariant 3 must still pass.
        let calldata = AddrTarget::invariant_addr_from_max_validCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_addr_from_max_valid must pass after actionAddrMaxValid"
        );
    }

    /// vm.addr(1) must return the same address when re-derived in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn addr_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let expected = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");

        // Re-derive in a new transaction.
        let calldata = AddrTarget::actionAddrOneCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionAddrOne must succeed");
        let stored = call_address_getter!(&mut chain, target, AddrTarget::addrFromOneCall);
        assert_eq!(
            stored, expected,
            "stored address must match after first action"
        );

        // Re-derive again in yet another transaction.
        let calldata = AddrTarget::actionAddrOneCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionAddrOne must succeed on second call");
        let stored = call_address_getter!(&mut chain, target, AddrTarget::addrFromOneCall);
        assert_eq!(
            stored, expected,
            "stored address must still match after second action"
        );
    }

    /// vm.addr(0) must revert at the handler level.
    #[test]
    fn addr_zero_key_reverts() {
        let outcome = addr::handle(U256::ZERO);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.addr(0) must revert with private-key-cannot-be-0"
        );
    }

    /// vm.addr with a key >= secp256k1 curve order must revert at the handler level.
    #[test]
    fn addr_key_too_large_reverts() {
        let bad_key = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ]);
        let outcome = addr::handle(bad_key);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.addr with key >= curve order must revert"
        );
    }

    /// The secp256k1 curve order used by `handle` must match the canonical
    /// constant exported by the `k256` crate.
    #[test]
    fn secp256k1_order_matches_k256_constant() {
        let order = U256::from_be_slice(&k256::Secp256k1::ORDER.to_be_byte_array());
        let expected = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ]);
        assert_eq!(order, expected);
    }
}
