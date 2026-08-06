// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ValidInvariantNonView {
    uint256 public value;

    event CheckFailed(string reason);

    function doSomething() external {
        value = 1;
    }

    /// Invariant functions may emit events for debugging, so they need not
    /// be view or pure. Storage writes are discarded because ripfuzz clones
    /// state before each fuzz input.
    function invariant_check() external {
        if (value > 1000) {
            emit CheckFailed("value too large");
            assert(false);
        }
    }
}
