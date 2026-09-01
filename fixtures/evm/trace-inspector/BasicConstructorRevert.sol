// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract BasicConstructorRevert {
    constructor() {
        revert("constructor always reverts");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
