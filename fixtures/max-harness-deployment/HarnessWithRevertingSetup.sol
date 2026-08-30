// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithRevertingSetup {
    uint256 internal stored;

    function setup() external {
        revert("setup failed");
    }

    function value() external view returns (uint256) {
        return stored;
    }
}
