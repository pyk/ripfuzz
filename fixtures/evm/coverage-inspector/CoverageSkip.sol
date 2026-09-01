// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// A contract with a function that returns early when a condition is not met,
/// simulating the `mintShouldSkip` pattern where a handler function bails out
/// before doing real work.
contract CoverageSkip {
    /// Returns early (no state change) when `x > 100`, otherwise stores it.
    /// This creates two distinct code paths: the "skip" path and the "work" path.
    function skipOrWork(uint256 x) external pure returns (uint256) {
        if (x > 100) return 0;
        return x + 1;
    }
}
