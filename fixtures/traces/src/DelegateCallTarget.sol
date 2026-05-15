// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract DelegateCallTarget {
    uint256 public value;

    function setValue(uint256 x) external {
        value = x;
    }
}
