// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract EmptyChainNoSetup {
    uint256 public value;

    constructor() {
        value = 42;
    }

    function doSomething() external {
        value = 1;
    }

    function invariant_check() public view {
        require(value >= 0, "ok");
    }
}
