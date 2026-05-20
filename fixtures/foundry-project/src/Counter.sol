// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract Counter {
    uint256 public count;

    function increment() external {
        count++;
    }

    function increment(uint256 amount) external {
        count += amount;
    }
}
