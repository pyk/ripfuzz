// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

error CustomErrorWithArgs(uint256 code, string message, address target);

contract CustomErrorWithArgsRevert {
    constructor() {
        revert CustomErrorWithArgs(42, "something went wrong", address(0x1234));
    }

    function set(uint256 x) external {
        // unreachable
    }
}
