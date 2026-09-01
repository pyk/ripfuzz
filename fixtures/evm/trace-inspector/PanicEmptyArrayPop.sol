// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract PanicEmptyArrayPop {
    uint256[] public arr;

    constructor() {
        arr.pop();
    }

    function set(uint256 x) external {
        // unreachable
    }
}
