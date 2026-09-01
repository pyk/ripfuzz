// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract PanicArithmeticOverflow {
    constructor() {
        uint256 x = type(uint256).max;
        x + 1;
    }

    function set(uint256 x) external {
        // unreachable
    }
}
