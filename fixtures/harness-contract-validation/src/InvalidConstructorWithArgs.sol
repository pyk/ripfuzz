// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidConstructorWithArgs {
    uint256 public value;

    constructor(uint256 x) {
        value = x;
    }

    function doSomething() external {
        value = 1;
    }
}
