// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithSetupAndSummary {
    uint256 public total;

    function setup() external {
        total = 10;
    }

    function deposit(uint256 amount) external {
        total += amount;
    }

    function summary() external view {
        require(total > 0, "empty");
    }
}
