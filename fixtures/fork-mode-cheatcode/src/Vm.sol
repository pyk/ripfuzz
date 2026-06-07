// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Raptor VM interface for cheatcodes.
interface Vm {
    function warp(uint256) external;
}
