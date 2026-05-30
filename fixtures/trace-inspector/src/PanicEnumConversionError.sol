// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract PanicEnumConversionError {
    enum Color { Red, Green, Blue }

    constructor() {
        uint256 n = 5;
        Color c = Color(n);
    }

    function set(uint256 x) external {
        // unreachable
    }
}
