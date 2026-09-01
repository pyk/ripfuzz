// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract PanicDivisionByZero {
    constructor() {
        uint256 x = 1;
        x / 0;
    }

    function set(uint256 x) external {
        // unreachable
    }
}