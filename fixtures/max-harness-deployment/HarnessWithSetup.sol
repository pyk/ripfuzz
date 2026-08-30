// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithSetup {
    uint256 internal stored;

    function setup() external {
        stored = 42;
    }

    function value() external view returns (uint256) {
        return stored;
    }
}
