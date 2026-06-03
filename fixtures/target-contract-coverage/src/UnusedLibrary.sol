// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library UnusedLibrary {
    function usedAdd(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function unusedAdd(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
