// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract NonceTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant NONCE_TARGET = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    address constant SEQUENCE_ADDR = 0x1111111111111111111111111111111111111111;
    uint64 constant EXPECTED_NONCE = 42;

    uint256 public storedNonce;

    function setup() external {
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        storedNonce = vm.getNonce(NONCE_TARGET);
    }

    function getStoredNonce() external view returns (uint256) {
        return storedNonce;
    }

    function getNonceExternal(address addr) external view returns (uint256) {
        return vm.getNonce(addr);
    }

    /// Call vm.setNonce with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callSetNonceSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        first = vm.getNonce(NONCE_TARGET);
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        second = vm.getNonce(NONCE_TARGET);
    }

    /// Call vm.setNonce with different values on a fresh account to prove
    /// sequence independence and value uniqueness.
    function callSetNonceSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.setNonce(SEQUENCE_ADDR, 1);
        first = vm.getNonce(SEQUENCE_ADDR);
        vm.setNonce(SEQUENCE_ADDR, EXPECTED_NONCE);
        second = vm.getNonce(SEQUENCE_ADDR);
        vm.setNonce(SEQUENCE_ADDR, 100);
        third = vm.getNonce(SEQUENCE_ADDR);
    }

    /// Interaction with deal - both cheatcodes in same tx.
    function callSetNonceAndDeal()
        external
        returns (uint256 nonce, uint256 balance)
    {
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        vm.deal(NONCE_TARGET, 1000 ether);
        nonce = vm.getNonce(NONCE_TARGET);
        balance = NONCE_TARGET.balance;
    }

    /// Setting nonce lower than current should revert.
    function callSetNonceAndRevertLowNonce() external {
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        vm.setNonce(NONCE_TARGET, 1); // This should revert
    }

    /// Fuzzing action: re-set the expected nonce and store it.
    function actionSetNonce() external {
        vm.setNonce(NONCE_TARGET, EXPECTED_NONCE);
        storedNonce = vm.getNonce(NONCE_TARGET);
    }

    function invariant_nonce() external view {
        assert(storedNonce == EXPECTED_NONCE);
    }
}
