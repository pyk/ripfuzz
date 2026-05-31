// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract EmptyChainSetupRevert {
    constructor() {}

    function setup() external pure {
        require(false, "setup reverted");
    }

    function doSomething() external {}
}
