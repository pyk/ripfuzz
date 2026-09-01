// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithSummary {
    uint256 public total;

    event Summarized(uint256 total);

    function deposit(uint256 amount) external {
        total += amount;
    }

    function summary() external {
        emit Summarized(total);
    }
}
