// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

import {BrokenInvariantError} from "../challenges/Challenge.sol";

contract HarnessWithFailingInvariant {
    uint256 public total;

    function increment(uint256 amount) external {
        total += amount;
    }

    function reset(uint256 value) external {
        total = value;
    }

    function invariant_total_below_limit() external {
        if (total > 100) {
            revert BrokenInvariantError({id: "INV-001", description: "total exceeded 100"});
        }
    }
}
