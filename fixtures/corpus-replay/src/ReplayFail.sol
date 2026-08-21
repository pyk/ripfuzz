// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness whose corpus already contains a failing sequence.
///
/// Replaying `trip()` then `invariant_never_tripped()` must surface a failed
/// assertion without needing any additional fuzzing runs.
contract ReplayFail {
    bool public tripped;

    function trip() external {
        tripped = true;
    }

    function invariant_never_tripped() external view {
        assert(!tripped);
    }
}
