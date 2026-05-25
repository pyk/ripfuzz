// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract AddrTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    /// secp256k1 curve order - 1 (largest valid private key).
    uint256 constant MAX_VALID_KEY =
        0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140;

    address public addrFromOne;
    address public addrFromTwo;
    address public addrFromMaxValid;

    function setup() external {
        addrFromOne = vm.addr(1);
        addrFromTwo = vm.addr(2);
        addrFromMaxValid = vm.addr(MAX_VALID_KEY);
    }

    function invariant_addr_from_one() external view {
        assert(addrFromOne == 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);
    }

    function invariant_addr_from_two() external view {
        assert(addrFromTwo == 0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF);
    }

    function invariant_addr_from_max_valid() external view {
        assert(addrFromMaxValid == 0x80C0dbf239224071c59dD8970ab9d542E3414aB2);
    }

    /// Call vm.addr(0) - must revert.
    function addrFromZero() external pure {
        vm.addr(0);
    }

    /// Call vm.addr with the curve order - must revert.
    function addrFromOrder() external pure {
        vm.addr(MAX_VALID_KEY + 1);
    }

    /// Getter for the max-valid-key address.
    function getAddrFromMaxValid() external view returns (address) {
        return addrFromMaxValid;
    }

    /// Call vm.addr(1) twice in the same transaction to prove determinism.
    function callAddrSameKeyTwice() external pure returns (address a, address b) {
        a = vm.addr(1);
        b = vm.addr(1);
    }

    /// Call vm.addr with different keys and interleave the same key
    /// to prove sequence independence and key uniqueness.
    function callAddrSequence()
        external
        pure
        returns (address first, address second, address third)
    {
        first = vm.addr(1);
        second = vm.addr(2);
        third = vm.addr(1);
    }

    // -----------------------------------------------------------------
    // Fuzzing-target action functions (called in call sequences)
    // -----------------------------------------------------------------

    /// Re-derive addr(1) and store it.  Fuzzer can call this in a sequence
    /// to prove `vm.addr` stays deterministic across transactions.
    function actionAddrOne() external {
        addrFromOne = vm.addr(1);
    }

    /// Re-derive addr(2) and store it.
    function actionAddrTwo() external {
        addrFromTwo = vm.addr(2);
    }

    /// Re-derive addr(max) and store it.
    function actionAddrMaxValid() external {
        addrFromMaxValid = vm.addr(MAX_VALID_KEY);
    }
}
