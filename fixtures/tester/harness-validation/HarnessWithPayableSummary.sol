// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithPayableSummary {
    function deposit(uint256 amount) external pure {
        require(amount > 0, "empty");
    }

    function summary() external payable {}
}
