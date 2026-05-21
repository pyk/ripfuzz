// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ValidNoInvariant {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }
}
