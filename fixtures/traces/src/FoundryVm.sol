// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Minimal Foundry VM interface for cheatcodes.
interface Vm {
    /// @notice Label an address for clearer traces.
    function label(address addr, string calldata name) external;
    /// @notice Set bytecode at an address (for pre-deploying contracts).
    function etch(address addr, bytes calldata code) external;
}
