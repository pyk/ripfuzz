// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract StaticCallTarget {
    uint256 public stored = 42;

    function getStored() external view returns (uint256) {
        return stored;
    }

    function getSum(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
