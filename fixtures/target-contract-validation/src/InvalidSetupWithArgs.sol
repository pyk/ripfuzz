// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidSetupWithArgs {
    uint256 public value;

    function setup(uint256 x) external {
        value = x;
    }

    function doSomething() external {
        value = 1;
    }
}
