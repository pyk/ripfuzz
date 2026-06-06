// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

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
