// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice A simple adder used as an external contract in fork-mode coverage tests.
contract Adder {
    /// @notice Add two numbers.
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        if (a > 0) {
            return a + b;
        }
        return b;
    }
}
