//! `sign` cheatcode - sign a digest with a private key.

use alloy_primitives::U256;
use k256::elliptic_curve::{Curve, bigint::ArrayEncoding};

use crate::evm::cheatcode::outcome;

pub fn handle(sk: U256, digest: [u8; 32]) -> Option<revm::interpreter::CallOutcome> {
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
    let (sig, recid) = signing_key.sign_prehash_recoverable(&digest).ok()?;
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    let v: u8 = if recid.is_y_odd() { 28 } else { 27 };
    let r_arr: [u8; 32] = AsRef::<[u8]>::as_ref(&r).try_into().ok()?;
    let s_arr: [u8; 32] = AsRef::<[u8]>::as_ref(&s).try_into().ok()?;
    Some(outcome::success_sign(v, r_arr, s_arr))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, U256, address};
    use alloy_sol_types::SolCall;
    use k256::elliptic_curve::{Curve, bigint::ArrayEncoding};
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployOptions, SetupOptions};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::Vm::signCall;
    use crate::evm::cheatcode::calls::sign;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface SignTarget {
            function signFromZero() external pure;
            function signFromOrder() external pure;
            function callSignSameKeyTwice()
                external
                pure
                returns (uint8 v1, bytes32 r1, bytes32 s1, uint8 v2, bytes32 r2, bytes32 s2);
            function callSignSequence()
                external
                pure
                returns (
                    uint8 v1,
                    bytes32 r1,
                    bytes32 s1,
                    uint8 v2,
                    bytes32 r2,
                    bytes32 s2,
                    uint8 v3,
                    bytes32 r3,
                    bytes32 s3
                );
            function callSignDifferentDigests()
                external
                pure
                returns (uint8 v1, bytes32 r1, bytes32 s1, uint8 v2, bytes32 r2, bytes32 s2);
            function callSignAndAddr()
                external
                pure
                returns (address derived, address recovered);
            function setup() external;
            function actionSignOne() external;
            function actionSignTwo() external;
            function actionSignMaxValid() external;
            function invariant_sign_from_one() external view;
            function invariant_sign_from_two() external view;
            function invariant_sign_from_max_valid() external view;
        }
    }

    /// Fixed digest used by the fixture contract.
    const DIGEST: B256 = B256::new([
        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        0xaa, 0xaa,
    ]);

    const ADDR_ONE: Address = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
    const ADDR_TWO: Address = address!("0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF");
    const ADDR_MAX_VALID: Address = address!("0x80C0dbf239224071c59dD8970ab9d542E3414aB2");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/SignTarget.sol:SignTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployOptions::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(SignTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupOptions::new(target, setup_data);
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

    /// Recover an address from a signature produced by `vm.sign`.
    fn recover_address(v: u8, r: &[u8; 32], s: &[u8; 32], digest: &B256) -> Address {
        let r_u256 = U256::from_be_bytes(*r);
        let s_u256 = U256::from_be_bytes(*s);
        let sig = alloy_primitives::Signature::new(r_u256, s_u256, v == 28);
        sig.recover_address_from_prehash(digest)
            .expect("valid signature must recover")
    }

    /// `vm.sign(1, digest)` at the handler level must recover to the well-known
    /// test address.
    #[test]
    fn sign_from_one_matches_expected() {
        let outcome = sign::handle(U256::from(1), DIGEST.into());
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.sign(1, digest) must succeed");

        let output = outcome.result.output;
        let ret = signCall::abi_decode_returns(&output).unwrap();
        let recovered = recover_address(ret.v, &ret.r.into(), &ret.s.into(), &DIGEST);
        assert_eq!(
            recovered, ADDR_ONE,
            "vm.sign(1) must recover to the well-known address"
        );
    }

    /// `vm.sign(2, digest)` at the handler level must recover to the second
    /// well-known test address.
    #[test]
    fn sign_from_two_matches_expected() {
        let outcome = sign::handle(U256::from(2), DIGEST.into());
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.sign(2, digest) must succeed");

        let output = outcome.result.output;
        let ret = signCall::abi_decode_returns(&output).unwrap();
        let recovered = recover_address(ret.v, &ret.r.into(), &ret.s.into(), &DIGEST);
        assert_eq!(
            recovered, ADDR_TWO,
            "vm.sign(2) must recover to the well-known address"
        );
    }

    /// `vm.sign(MAX_VALID_KEY, digest)` at the handler level must recover to
    /// the expected address.
    #[test]
    fn sign_from_max_valid_key_matches_expected() {
        let max_valid =
            U256::from_be_slice(&k256::Secp256k1::ORDER.to_be_byte_array()) - U256::from(1);
        let outcome = sign::handle(max_valid, DIGEST.into());
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            outcome.result.is_ok(),
            "vm.sign(max_valid, digest) must succeed"
        );

        let output = outcome.result.output;
        let ret = signCall::abi_decode_returns(&output).unwrap();
        let recovered = recover_address(ret.v, &ret.r.into(), &ret.s.into(), &DIGEST);
        assert_eq!(
            recovered, ADDR_MAX_VALID,
            "vm.sign(max_valid) must recover to the well-known address"
        );
    }

    /// Calling `signFromZero()` must revert because `vm.sign(0, digest)` reverts.
    #[test]
    fn sign_zero_key_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::signFromZeroCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "signFromZero() must revert when vm.sign(0) is called"
        );
    }

    /// Calling `signFromOrder()` must revert because `vm.sign(order, digest)` reverts.
    #[test]
    fn sign_order_key_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::signFromOrderCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "signFromOrder() must revert when vm.sign(order) is called"
        );
    }

    /// `vm.sign(1, digest)` called twice in the same transaction must return the
    /// same signature, proving the cheatcode is deterministic and stateless.
    #[test]
    fn sign_same_key_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::callSignSameKeyTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSignSameKeyTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = SignTarget::callSignSameKeyTwiceCall::abi_decode_returns(&output).unwrap();

        assert_eq!(ret.v1, ret.v2, "v must match for identical inputs");
        assert_eq!(ret.r1, ret.r2, "r must match for identical inputs");
        assert_eq!(ret.s1, ret.s2, "s must match for identical inputs");

        let recovered = recover_address(ret.v1, &ret.r1.into(), &ret.s1.into(), &DIGEST);
        assert_eq!(recovered, ADDR_ONE, "signature must recover to addr(1)");
    }

    /// `vm.sign` must return the same signature for the same key even when
    /// interleaved with calls for different keys, and different keys must
    /// produce different signatures.
    #[test]
    fn sign_sequence_returns_consistent_and_unique_signatures() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::callSignSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSignSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = SignTarget::callSignSequenceCall::abi_decode_returns(&output).unwrap();

        // First and third calls used the same key -> identical signatures.
        assert_eq!(ret.v1, ret.v3, "same key must give same v");
        assert_eq!(ret.r1, ret.r3, "same key must give same r");
        assert_eq!(ret.s1, ret.s3, "same key must give same s");

        // Different keys must produce different signatures.
        assert!(
            ret.r1 != ret.r2 || ret.s1 != ret.s2,
            "different keys must produce different signatures"
        );

        let recovered_one = recover_address(ret.v1, &ret.r1.into(), &ret.s1.into(), &DIGEST);
        let recovered_two = recover_address(ret.v2, &ret.r2.into(), &ret.s2.into(), &DIGEST);
        assert_eq!(recovered_one, ADDR_ONE);
        assert_eq!(recovered_two, ADDR_TWO);
    }

    /// Different digests with the same key must produce different signatures.
    #[test]
    fn sign_different_digests_produce_different_signatures() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::callSignDifferentDigestsCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSignDifferentDigests() must succeed");
        let output = result.output.expect("must return output");
        let ret = SignTarget::callSignDifferentDigestsCall::abi_decode_returns(&output).unwrap();

        assert!(
            ret.r1 != ret.r2 || ret.s1 != ret.s2,
            "different digests must produce different signatures"
        );

        let recovered_one = recover_address(ret.v1, &ret.r1.into(), &ret.s1.into(), &DIGEST);
        let other_digest = B256::from(U256::from_be_bytes(DIGEST.into()) + U256::from(1));
        let recovered_two = recover_address(ret.v2, &ret.r2.into(), &ret.s2.into(), &other_digest);
        assert_eq!(recovered_one, ADDR_ONE);
        assert_eq!(
            recovered_two, ADDR_ONE,
            "same key must recover for both digests"
        );
    }

    /// The signature derived during setup must still verify in a later
    /// transaction, proving contract-level persistence works.
    #[test]
    fn sign_setup_value_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();

        let invariants = [
            (
                SignTarget::invariant_sign_from_oneCall::new(()).abi_encode(),
                "invariant_sign_from_one",
            ),
            (
                SignTarget::invariant_sign_from_twoCall::new(()).abi_encode(),
                "invariant_sign_from_two",
            ),
            (
                SignTarget::invariant_sign_from_max_validCall::new(()).abi_encode(),
                "invariant_sign_from_max_valid",
            ),
        ];
        for (calldata, name) in &invariants {
            let result = chain
                .call(
                    DEFAULT_DEPLOYER,
                    target,
                    U256::ZERO,
                    Bytes::from(calldata.clone()),
                )
                .unwrap();
            assert!(result.success, "{name} must pass after setup (first call)");
        }
        for (calldata, name) in &invariants {
            let result = chain
                .call(
                    DEFAULT_DEPLOYER,
                    target,
                    U256::ZERO,
                    Bytes::from(calldata.clone()),
                )
                .unwrap();
            assert!(result.success, "{name} must pass after setup (second call)");
        }
    }

    /// `vm.sign` and `vm.addr` must interact correctly: the address derived by
    /// `addr(1)` must match the address recovered from `sign(1, digest)`.
    #[test]
    fn sign_interacts_with_addr() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = SignTarget::callSignAndAddrCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSignAndAddr() must succeed");
        let output = result.output.expect("must return output");
        let ret = SignTarget::callSignAndAddrCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.derived, ret.recovered,
            "vm.addr(1) must match ecrecover(vm.sign(1, digest))"
        );
        assert_eq!(ret.derived, ADDR_ONE);
    }

    /// Invariants must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let invariants = [
            (
                SignTarget::invariant_sign_from_oneCall::new(()).abi_encode(),
                "invariant_sign_from_one",
            ),
            (
                SignTarget::invariant_sign_from_twoCall::new(()).abi_encode(),
                "invariant_sign_from_two",
            ),
            (
                SignTarget::invariant_sign_from_max_validCall::new(()).abi_encode(),
                "invariant_sign_from_max_valid",
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
    /// This proves `vm.sign` stays deterministic across multiple transactions
    /// and that invariants correctly observe the persisted state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Action 1: re-sign with key 1 and store it.
        let calldata = SignTarget::actionSignOneCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSignOne must succeed");

        let calldata = SignTarget::invariant_sign_from_oneCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_sign_from_one must pass after actionSignOne"
        );

        // Action 2: re-sign with key 2 and store it.
        let calldata = SignTarget::actionSignTwoCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSignTwo must succeed");

        let calldata = SignTarget::invariant_sign_from_twoCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_sign_from_two must pass after actionSignTwo"
        );

        // Action 3: re-sign with max valid key and store it.
        let calldata = SignTarget::actionSignMaxValidCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSignMaxValid must succeed");

        let calldata = SignTarget::invariant_sign_from_max_validCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(
            result.success,
            "invariant_sign_from_max_valid must pass after actionSignMaxValid"
        );
    }

    /// `vm.sign(1, digest)` must return the same signature when re-derived in a
    /// separate transaction after the initial setup, proving cross-transaction
    /// determinism.
    #[test]
    fn sign_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        // Re-sign in a new transaction.
        let calldata = SignTarget::actionSignOneCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionSignOne must succeed");
        let calldata_inv = SignTarget::invariant_sign_from_oneCall::new(()).abi_encode();
        let result = chain
            .call(
                DEFAULT_DEPLOYER,
                target,
                U256::ZERO,
                Bytes::from(calldata_inv.clone()),
            )
            .unwrap();
        assert!(result.success, "invariant must pass after first action");

        // Re-sign again in yet another transaction.
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSignOne must succeed on second call");
        let result = chain
            .call(
                DEFAULT_DEPLOYER,
                target,
                U256::ZERO,
                Bytes::from(calldata_inv),
            )
            .unwrap();
        assert!(
            result.success,
            "invariant must still pass after second action"
        );
    }

    /// `vm.sign(0, digest)` must revert at the handler level.
    #[test]
    fn sign_zero_key_reverts() {
        let outcome = sign::handle(U256::ZERO, DIGEST.into());
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.sign(0) must revert with private-key-cannot-be-0"
        );
    }

    /// `vm.sign` with a key >= secp256k1 curve order must revert at the handler level.
    #[test]
    fn sign_key_too_large_reverts() {
        let bad_key = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ]);
        let outcome = sign::handle(bad_key, DIGEST.into());
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(
            !outcome.result.is_ok(),
            "vm.sign with key >= curve order must revert"
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
