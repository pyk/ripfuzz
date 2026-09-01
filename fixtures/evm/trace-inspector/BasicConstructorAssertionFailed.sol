// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract BasicConstructorAssertionFailed {
    constructor() {
        assert(false);
    }

    function set(uint256 x) external {
        // unreachable
    }
}
