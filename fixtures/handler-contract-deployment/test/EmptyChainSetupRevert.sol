// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract EmptyChainSetupRevert {
    uint256 public value;

    constructor() {}

    function setup() external {
        value = 1;
        require(false, "setup reverted");
    }

    function doSomething() external {}
}
