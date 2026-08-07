// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz RVM interface for cheatcodes.
///
/// NOTE: The ripfuzz RVM is **not** Foundry VM compatible.  It does not
/// implement all Foundry cheatcodes - only the subset supported by ripfuzz.
interface RVM {
    function label(address, string calldata) external;
    function prank(address) external;
    function startPrank(address) external;
    function stopPrank() external;
    function deal(address, uint256) external;
    function warp(uint256) external;
    function roll(uint256) external;
    function ffi(string[] calldata) external returns (bytes memory);
    function getCode(string calldata) external returns (bytes memory);
    function getEnv(string calldata) external returns (string memory);
    function fork(string calldata url, uint256 blockNumber) external;
}
