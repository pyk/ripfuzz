// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Raptor VM interface for cheatcodes.
///
/// NOTE: The raptor VM is **not** Foundry VM compatible.  It does not
/// implement all Foundry cheatcodes — only the subset supported by raptor.
interface Vm {
    function warp(uint256) external;
    function label(address, string calldata) external;
    function startPrank(address) external;
    function stopPrank() external;
    function snapshot() external returns (uint256);
    function revertTo(uint256) external returns (bool);
}
