// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract PanicArrayOutOfBounds {
    uint256[] public arr;

    constructor() {
        arr[0];
    }

    function set(uint256 x) external {
        // unreachable
    }
}