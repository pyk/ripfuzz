// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract ValidSummary {
    function touch() external {}

    event Summarized(uint256 value);

    function summary() external {
        emit Summarized(42);
    }
}
