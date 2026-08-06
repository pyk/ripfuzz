// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidSetupView {
    uint256 public value;

    function setup() external view {
        require(value == 0, "setup");
    }

    function doSomething() external {
        value = 1;
    }
}
