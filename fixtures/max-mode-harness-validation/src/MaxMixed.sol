// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract that declares both invariant and max functions.
///
/// This is invalid: `max_value()` puts the harness in max mode automatically,
/// and max mode rejects `invariant_*` functions.
contract MaxMixed {
    uint256 internal value;

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
