// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Minimal ripfuzz RVM interface for cheatcodes.
interface RVM {
    function warp(uint256) external;
}
