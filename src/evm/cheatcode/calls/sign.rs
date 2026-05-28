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

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::Vm::signCall;
    use crate::evm::cheatcode::calls::sign;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface SignTarget {
            function setup() external;
            function actionResignOne() external;
            function actionResignTwo() external;
            function actionResignMaxValid() external;
            function actionSignZero() external pure;
            function actionSignOrder() external pure;
            function actionSignAndAddr() external pure returns (address derived, address recovered);
            function invariant_sigOneValid() external view;
            function invariant_sigTwoValid() external view;
            function invariant_sigMaxValid() external view;
        }
    }

    const DIGEST: B256 = B256::new([
        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        0xaa, 0xaa,
    ]);

    const ADDR_ONE: Address = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf");
    const ADDR_TWO: Address = address!("0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF");
    const ADDR_MAX: Address = address!("0x80C0dbf239224071c59dD8970ab9d542E3414aB2");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/SignTarget.sol:SignTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

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
            recovered, ADDR_MAX,
            "vm.sign(max_valid) must recover to the well-known address"
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

    /// `vm.sign` used during setup must store signatures that recover to the
    /// well-known addresses. The invariants check that all three keys produce
    /// valid signatures.
    #[test]
    fn setup_derives_well_known_signatures() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigTwoValidCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigMaxValidCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "invariant_sigOneValid must pass after setup"
        );
        assert!(
            execution.results[1].success,
            "invariant_sigTwoValid must pass after setup"
        );
        assert!(
            execution.results[2].success,
            "invariant_sigMaxValid must pass after setup"
        );
    }

    /// Re-signing with the same key in a later transaction and overwriting
    /// storage must still yield a signature that recovers to the same address.
    /// This is the core property a stateful fuzzer relies on.
    #[test]
    fn re_sign_in_action_preserves_validity() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignOneCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionResignOne must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after re-sign"
        );
    }

    /// A single transaction can re-sign with multiple keys without corrupting
    /// results. This proves `vm.sign` is stateless and safe to call repeatedly
    /// inside one tx.
    #[test]
    fn batch_re_sign_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignOneCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignTwoCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignMaxValidCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigTwoValidCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigMaxValidCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 6);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all batch steps must succeed"
        );
    }

    /// `vm.sign(0, digest)` must revert because 0 is not a valid private key.
    #[test]
    fn invalid_zero_key_reverts_in_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            SignTarget::actionSignZeroCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.sign(0) must revert in a transaction"
        );
    }

    /// `vm.sign` with a key >= secp256k1 curve order must revert.
    #[test]
    fn invalid_order_key_reverts_in_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            SignTarget::actionSignOrderCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.sign(order) must revert in a transaction"
        );
    }

    /// `vm.sign` and `vm.addr` must interact correctly: the address derived by
    /// `addr(1)` must match the address recovered from `sign(1, digest)`.
    #[test]
    fn sign_and_addr_agree() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            SignTarget::actionSignAndAddrCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "actionSignAndAddr must succeed"
        );
        let output = execution.results[0]
            .output
            .clone()
            .expect("must return output");
        let ret = SignTarget::actionSignAndAddrCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.derived, ret.recovered,
            "vm.addr(1) must match ecrecover(vm.sign(1, digest))"
        );
        assert_eq!(
            ret.derived, ADDR_ONE,
            "derived address must be the well-known addr(1)"
        );
    }

    /// A cloned chain snapshot must produce the same signatures when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_produces_same_signatures() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignOneCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
        ];
        let execution = cloned.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionResignOne must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// Cross-transaction determinism: re-signing in a second `exec` must still
    /// produce a valid signature that recovers to the same address.
    #[test]
    fn deterministic_across_separate_execs() {
        let (mut chain, target) = deploy_and_setup();

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignOneCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "first exec must succeed"
        );

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::actionResignOneCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                SignTarget::invariant_sigOneValidCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "second exec must succeed"
        );
    }
}
