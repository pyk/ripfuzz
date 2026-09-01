// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithSummaryArgs {
    function deposit(uint256 amount) external pure {
        require(amount > 0, "empty");
    }

    function summary(uint256 extra) external pure {
        require(extra > 0, "empty");
    }
}
