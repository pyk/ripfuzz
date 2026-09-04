// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithConstructorArgs {
    constructor(uint256 start) {
        require(start > 0, "empty");
    }

    function deposit(uint256 amount) external pure {
        require(amount > 0, "empty");
    }
}
