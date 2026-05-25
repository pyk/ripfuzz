// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract SignTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    /// secp256k1 curve order - 1 (largest valid private key).
    uint256 constant MAX_VALID_KEY =
        0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140;

    /// Fixed digest used for deterministic signature tests.
    bytes32 constant DIGEST =
        0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;

    /// Well-known address derived from private key 1.
    address constant ADDR_ONE = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;

    uint8 public vFromOne;
    bytes32 public rFromOne;
    bytes32 public sFromOne;

    uint8 public vFromTwo;
    bytes32 public rFromTwo;
    bytes32 public sFromTwo;

    uint8 public vFromMaxValid;
    bytes32 public rFromMaxValid;
    bytes32 public sFromMaxValid;

    function setup() external {
        (vFromOne, rFromOne, sFromOne) = vm.sign(1, DIGEST);
        (vFromTwo, rFromTwo, sFromTwo) = vm.sign(2, DIGEST);
        (vFromMaxValid, rFromMaxValid, sFromMaxValid) = vm.sign(MAX_VALID_KEY, DIGEST);
    }

    function invariant_sign_from_one() external view {
        address recovered = ecrecover(DIGEST, vFromOne, rFromOne, sFromOne);
        assert(recovered == ADDR_ONE);
    }

    function invariant_sign_from_two() external view {
        address recovered = ecrecover(DIGEST, vFromTwo, rFromTwo, sFromTwo);
        assert(recovered == 0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF);
    }

    function invariant_sign_from_max_valid() external view {
        address recovered = ecrecover(DIGEST, vFromMaxValid, rFromMaxValid, sFromMaxValid);
        assert(recovered == 0x80C0dbf239224071c59dD8970ab9d542E3414aB2);
    }

    /// Call vm.sign(0, digest) - must revert.
    function signFromZero() external pure {
        vm.sign(0, DIGEST);
    }

    /// Call vm.sign with the curve order - must revert.
    function signFromOrder() external pure {
        vm.sign(MAX_VALID_KEY + 1, DIGEST);
    }

    /// Call vm.sign(1, digest) twice in the same transaction to prove determinism.
    function callSignSameKeyTwice()
        external
        pure
        returns (uint8 v1, bytes32 r1, bytes32 s1, uint8 v2, bytes32 r2, bytes32 s2)
    {
        (v1, r1, s1) = vm.sign(1, DIGEST);
        (v2, r2, s2) = vm.sign(1, DIGEST);
    }

    /// Call vm.sign with different keys and interleave the same key
    /// to prove sequence independence and key uniqueness.
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
        )
    {
        (v1, r1, s1) = vm.sign(1, DIGEST);
        (v2, r2, s2) = vm.sign(2, DIGEST);
        (v3, r3, s3) = vm.sign(1, DIGEST);
    }

    /// Call vm.sign with different digests for the same key
    /// to prove digest sensitivity.
    function callSignDifferentDigests()
        external
        pure
        returns (
            uint8 v1,
            bytes32 r1,
            bytes32 s1,
            uint8 v2,
            bytes32 r2,
            bytes32 s2
        )
    {
        (v1, r1, s1) = vm.sign(1, DIGEST);
        (v2, r2, s2) = vm.sign(1, bytes32(uint256(DIGEST) + 1));
    }

    /// Interaction with vm.addr: derive address from key, then sign and verify.
    function callSignAndAddr()
        external
        pure
        returns (address derived, address recovered)
    {
        derived = vm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, DIGEST);
        recovered = ecrecover(DIGEST, v, r, s);
    }

    // -----------------------------------------------------------------
    // Fuzzing-target action functions (called in call sequences)
    // -----------------------------------------------------------------

    /// Re-sign with key 1 and store it. Fuzzer can call this in a sequence
    /// to prove `vm.sign` stays deterministic across transactions.
    function actionSignOne() external {
        (vFromOne, rFromOne, sFromOne) = vm.sign(1, DIGEST);
    }

    /// Re-sign with key 2 and store it.
    function actionSignTwo() external {
        (vFromTwo, rFromTwo, sFromTwo) = vm.sign(2, DIGEST);
    }

    /// Re-sign with max valid key and store it.
    function actionSignMaxValid() external {
        (vFromMaxValid, rFromMaxValid, sFromMaxValid) = vm.sign(MAX_VALID_KEY, DIGEST);
    }
}
