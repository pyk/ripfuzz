// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

error CustomConstructorError();

contract BasicConstructorCustomErrorRevert {
    constructor() {
        revert CustomConstructorError();
    }

    function set(uint256 x) external {
        // unreachable
    }
}
