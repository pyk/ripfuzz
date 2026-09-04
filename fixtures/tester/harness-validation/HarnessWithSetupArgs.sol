// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithSetupArgs {
    function setup(uint256 start) external {}

    function deposit(uint256 amount) external pure {
        require(amount > 0, "empty");
    }
}
