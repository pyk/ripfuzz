// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz VM interface for cheatcodes.
///
/// NOTE: The ripfuzz VM is **not** Foundry VM compatible.  It does not
/// implement all Foundry cheatcodes — only the subset supported by ripfuzz.
interface Vm {
    function warp(uint256) external;
    function label(address, string calldata) external;
    function startPrank(address) external;
    function stopPrank() external;
    function snapshot() external returns (uint256);
    function revertTo(uint256) external returns (bool);
}
