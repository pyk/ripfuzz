// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz RVM interface for cheatcodes.
interface RVM {
    function fork(string calldata url, uint256 blockNumber) external;
}
