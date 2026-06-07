// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Regression fixture for a bug where deploying a basic contract
/// in fork mode caused an unnecessary RPC fetch for the newly created address.
contract BasicContract {
    uint256 public value;

    constructor() {
        value = 42;
    }

    function setValue(uint256 newValue) external {
        value = newValue;
    }

    function invariant_value_not_zero() external view {
        assert(value > 0);
    }
}
