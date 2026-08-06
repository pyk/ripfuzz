// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice Interface for the external Adder contract.
interface IAdder {
    function add(uint256 a, uint256 b) external view returns (uint256);
}

/// @notice Harness contract that calls an external Adder via fork mode.
contract MultiProjectHarness {
    /// @notice Call the external Adder at the given address.
    function callAdder(address adder) external view returns (uint256) {
        return IAdder(adder).add(1, 2);
    }
}
