// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A target contract used to test system-level invariant detection.
///
/// The bug lives inside the `invariant_step_not_three()` function: after
/// calling `advance()` three times, `step` equals `3`, causing the
/// invariant to fail.  The minimal reproducing sequence is exactly
/// `advance() -> advance() -> advance()`.
contract SystemLevelInvariant {
    uint256 public step;

    function advance() external {
        step += 1;
    }

    function invariant_step_not_three() external view {
        assert(step < 3);
    }
}
