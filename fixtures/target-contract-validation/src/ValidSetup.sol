// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ValidSetup {
    uint256 public value;

    function setup() external {
        value = 1;
    }

    function doSomething() external {
        value = 2;
    }
}
