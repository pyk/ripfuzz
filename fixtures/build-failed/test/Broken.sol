// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Broken {
    uint256 public value

    function set(uint256 x) external {
        value = x
    }
}
