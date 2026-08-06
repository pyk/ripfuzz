// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz VM interface for cheatcodes.
///
/// NOTE: The ripfuzz VM is **not** Foundry VM compatible.  It does not
/// implement all Foundry cheatcodes — only the subset supported by ripfuzz.
interface Vm {
    function label(address, string calldata) external;
    function prank(address) external;
    function startPrank(address) external;
    function stopPrank() external;
    function deal(address, uint256) external;
    function warp(uint256) external;
    function roll(uint256) external;
    function ffi(string[] calldata) external returns (bytes memory);
    function getCode(string calldata) external returns (bytes memory);
}
