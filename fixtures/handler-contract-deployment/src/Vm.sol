// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Minimal raptor VM interface for cheatcodes.
interface Vm {
    function warp(uint256) external;
}
