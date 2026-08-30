// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithNoImports {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function getValue() external view returns (uint256) {
        return value;
    }
}
