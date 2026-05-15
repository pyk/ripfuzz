// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Target {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function check() external view {
        assert(value != 0xdead);
    }
}
