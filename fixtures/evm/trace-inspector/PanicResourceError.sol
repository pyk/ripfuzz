// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract PanicResourceError {
    constructor() {
        uint256[] memory arr = new uint256[](2**64);
    }

    function set(uint256 x) external {
        // unreachable
    }
}
