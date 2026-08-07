// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz Virtual Machine interface for cheatcodes.
///
/// @dev The ripfuzz RVM is not Foundry-compatible. It implements only the
/// cheatcode subset supported by ripfuzz.
interface RVM {
    function warp(uint256) external;
    function label(address, string calldata) external;
    function startPrank(address) external;
    function stopPrank() external;
    function snapshot() external returns (uint256);
    function revertTo(uint256) external returns (bool);
}
