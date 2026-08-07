// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice Ripfuzz RVM interface for cheatcodes.
interface RVM {
    function fork(string calldata url, uint256 blockNumber) external;
}

/// @notice Interface for the external Adder contract.
interface IAdder {
    function add(uint256 a, uint256 b) external view returns (uint256);
}

/// @notice Harness contract that calls an external Adder via fork mode.
contract MultiProjectHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// @notice Enter fork mode at the pinned block used by the test mocks.
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
    }

    /// @notice Call the external Adder at the given address.
    function callAdder(address adder) external view returns (uint256) {
        return IAdder(adder).add(1, 2);
    }
}
