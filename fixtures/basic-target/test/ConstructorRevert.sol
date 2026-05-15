// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ConstructorRevert {
    constructor() {
        revert("constructor always reverts");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
