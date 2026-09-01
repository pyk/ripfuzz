// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithFailingInvariant {
    uint256 public total;

    function increment(uint256 amount) external {
        total += amount;
    }

    function reset(uint256 value) external {
        total = value;
    }

    function invariant_total_below_limit() external view {
        assert(total <= 100);
    }
}
