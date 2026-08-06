// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice A contract with an immutable variable to test coverage matching.
contract CoverageImmutable {
    /// @notice The immutable value.
    uint256 public immutable value;

    /// @notice Constructor that sets the immutable value.
    constructor() {
        value = 42;
    }

    /// @notice A function that returns the immutable value.
    function getValue() external view returns (uint256) {
        return value;
    }
}
