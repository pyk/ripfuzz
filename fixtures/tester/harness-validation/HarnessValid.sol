// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessValid {
    uint256 public total;

    function deposit(uint256 amount) external {
        total += amount;
    }

    function invariant_total() external view {
        assert(total < type(uint256).max);
    }
}
