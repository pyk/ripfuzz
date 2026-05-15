// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

contract Original {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function property_is_set() external view returns (bool) {
        return value > 0;
    }
}
