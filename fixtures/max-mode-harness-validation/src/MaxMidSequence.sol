// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract where the best value is achieved by a prefix.
///
/// `set()` raises the value and `clear()` resets it to zero. The best sequence
/// for `max_value()` is the shortest prefix ending in `set()`, and the shrinker
/// must remove the trailing `clear()`.
contract MaxMidSequence {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function clear() external {
        value = 0;
    }

    function max_value() external view returns (uint256) {
        return value;
    }
}
