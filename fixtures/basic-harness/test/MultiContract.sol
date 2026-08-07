// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

contract A {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }
}

contract B {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }
}
