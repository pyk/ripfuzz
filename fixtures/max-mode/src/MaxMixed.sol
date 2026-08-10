// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract that declares both invariant and max functions.
///
/// In invariant mode `max_value()` must not be treated as a handler, and the
/// invariant fails on any nonzero value. In max mode the invariant must be
/// ignored and `max_value()` is maximized instead.
contract MaxMixed {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function invariant_value_is_zero() external view {
        assert(value == 0);
    }

    function max_value() external view returns (uint256) {
        return value;
    }
}
