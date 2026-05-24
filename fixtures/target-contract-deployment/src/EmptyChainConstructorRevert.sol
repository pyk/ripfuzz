// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract EmptyChainConstructorRevert {
    constructor() {
        require(false, "constructor reverted");
    }

    function doSomething() external {}
}
