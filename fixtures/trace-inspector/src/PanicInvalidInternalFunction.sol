// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract PanicInvalidInternalFunction {
    function() internal fn;

    constructor() {
        fn();
    }

    function set(uint256 x) external {
        // unreachable
    }
}
