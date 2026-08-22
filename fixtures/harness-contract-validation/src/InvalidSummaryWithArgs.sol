// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract InvalidSummaryWithArgs {
    function touch() external {}

    event Summarized(uint256 value);

    function summary(uint256 value) external {
        emit Summarized(value);
    }
}
