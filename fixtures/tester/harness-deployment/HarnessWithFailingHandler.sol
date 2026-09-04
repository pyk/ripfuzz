// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BrokenInvariantError} from "../challenges/Challenge.sol";

contract HarnessWithFailingHandler {
    uint256 public total;

    function deposit(uint256 amount) external {
        total += amount;
        if (total >= 1000) {
            revert BrokenInvariantError({id: "HAN-001", description: "total exceeded 1000"});
        }
    }

    function invariant_total() external {
        if (total >= type(uint256).max) {
            revert BrokenInvariantError({id: "INV-MAX", description: "total overflowed"});
        }
    }
}
