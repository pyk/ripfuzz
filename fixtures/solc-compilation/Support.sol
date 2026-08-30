// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract Support {
    uint256 public stored;

    function setValue(uint256 x) external {
        stored = x;
    }
}
