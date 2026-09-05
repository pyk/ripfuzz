// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title Base fixture contract
/// @notice Holds the total the app reads and writes.
contract Base {
    /// @notice The current total.
    uint256 public total;

    function setValue(uint256 newTotal) external {
        total = newTotal;
    }
}
