// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithSetupAndSummary {
    uint256 internal stored;

    function setup() external {
        stored = 1;
    }

    function value() external view returns (uint256) {
        return stored;
    }

    function summary() external view returns (string memory) {
        return "done";
    }
}
