// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Base {
    uint256 public total;

    function setValue(uint256 newTotal) external {
        total = newTotal;
    }
}
