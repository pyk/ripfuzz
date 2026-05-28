// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library MathLibExternal {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
