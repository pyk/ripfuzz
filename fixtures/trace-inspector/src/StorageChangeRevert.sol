// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract StorageChangeRevert {
    uint256 public value;

    constructor() {
        value = 42;
        revert("constructor reverted after storage write");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
