// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Ripfuzz RVM interface for cheatcodes.
interface RVM {
    function store(address, bytes32, bytes32) external;
    function load(address, bytes32) external view returns (bytes32);
    function label(address, string calldata) external;
    function prank(address) external;
    function startPrank(address) external;
    function stopPrank() external;
    function fork(string calldata url, uint256 blockNumber) external;
}
