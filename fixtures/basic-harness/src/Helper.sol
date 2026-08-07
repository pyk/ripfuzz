// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Helper {
    uint256 public value;

    function setValue(uint256 x) external {
        require(x > 100, "value too low");
        value = x;
    }
}
