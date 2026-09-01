// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithFailingHandler {
    uint256 public total;

    function deposit(uint256 amount) external {
        total += amount;
        assert(total < 1000);
    }

    function invariant_total() external view {
        assert(total < type(uint256).max);
    }
}
