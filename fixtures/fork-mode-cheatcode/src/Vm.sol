// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Raptor VM interface for cheatcodes.
interface Vm {
    function warp(uint256) external;
    function roll(uint256) external;
    function chainId(uint256) external;
    function addr(uint256) external pure returns (address);
    function deal(address, uint256) external;
    function load(address, bytes32) external returns (bytes32);
    function store(address, bytes32, bytes32) external;
}
