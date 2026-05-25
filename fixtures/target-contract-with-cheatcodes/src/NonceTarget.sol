// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract NonceTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant ACTOR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    uint64 constant BASELINE = 42;

    uint256 public storedNonce;

    function setup() external {
        vm.setNonce(ACTOR, BASELINE);
        storedNonce = vm.getNonce(ACTOR);
    }

    /// Bump the actor nonce by one and store it.
    function actionBumpNonce() external {
        uint64 current = uint64(vm.getNonce(ACTOR));
        vm.setNonce(ACTOR, current + 1);
        storedNonce = vm.getNonce(ACTOR);
    }

    /// Bump the actor nonce by two and store it.
    function actionBumpNonceByTwo() external {
        uint64 current = uint64(vm.getNonce(ACTOR));
        vm.setNonce(ACTOR, current + 2);
        storedNonce = vm.getNonce(ACTOR);
    }

    /// Overwrite the actor nonce multiple times, ending +30 above current.
    function actionOverwriteSequence() external {
        uint64 current = uint64(vm.getNonce(ACTOR));
        vm.setNonce(ACTOR, current + 10);
        vm.setNonce(ACTOR, current + 20);
        vm.setNonce(ACTOR, current + 30);
        storedNonce = vm.getNonce(ACTOR);
    }

    /// Attempt to set nonce lower than current. Must revert.
    function actionRevertLowNonce() external {
        uint64 current = uint64(vm.getNonce(ACTOR));
        vm.setNonce(ACTOR, current - 1);
    }

    function getStoredNonce() external view returns (uint256) {
        return storedNonce;
    }

    /// Read the actor nonce directly from the cheatcode inspector.
    /// Used to prove that vm.setNonce in setup persists into exec.
    function getNonceDirect() external view returns (uint256) {
        return vm.getNonce(ACTOR);
    }

    /// Invariant: the stored nonce must never drop below the baseline.
    function invariant_nonceAtLeastBaseline() external view {
        assert(storedNonce >= BASELINE);
    }
}
