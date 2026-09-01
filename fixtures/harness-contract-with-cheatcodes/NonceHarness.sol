// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";

contract NonceHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address constant ACTOR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    uint64 constant BASELINE = 42;

    uint256 public storedNonce;

    function setup() external {
        rvm.setNonce(ACTOR, BASELINE);
        storedNonce = rvm.getNonce(ACTOR);
    }

    /// Bump the actor nonce by one and store it.
    function actionBumpNonce() external {
        uint64 current = uint64(rvm.getNonce(ACTOR));
        rvm.setNonce(ACTOR, current + 1);
        storedNonce = rvm.getNonce(ACTOR);
    }

    /// Bump the actor nonce by two and store it.
    function actionBumpNonceByTwo() external {
        uint64 current = uint64(rvm.getNonce(ACTOR));
        rvm.setNonce(ACTOR, current + 2);
        storedNonce = rvm.getNonce(ACTOR);
    }

    /// Overwrite the actor nonce multiple times, ending +30 above current.
    function actionOverwriteSequence() external {
        uint64 current = uint64(rvm.getNonce(ACTOR));
        rvm.setNonce(ACTOR, current + 10);
        rvm.setNonce(ACTOR, current + 20);
        rvm.setNonce(ACTOR, current + 30);
        storedNonce = rvm.getNonce(ACTOR);
    }

    /// Attempt to set nonce lower than current. Must revert.
    function actionRevertLowNonce() external {
        uint64 current = uint64(rvm.getNonce(ACTOR));
        rvm.setNonce(ACTOR, current - 1);
    }

    function getStoredNonce() external view returns (uint256) {
        return storedNonce;
    }

    /// Read the actor nonce directly from the cheatcode inspector.
    /// Used to prove that rvm.setNonce in setup persists into exec.
    function getNonceDirect() external view returns (uint256) {
        return rvm.getNonce(ACTOR);
    }

    /// Invariant: the stored nonce must never drop below the baseline.
    function invariant_nonceAtLeastBaseline() external view {
        assert(storedNonce >= BASELINE);
    }
}
