// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.warp`
/// cheatcode correctly updates `block.timestamp` in fork mode.
contract RvmWarp {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Enter fork mode at the pinned mainnet block used by the test mocks.
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
    }

    /// Warp to `block.timestamp + value`.
    function warp(uint256 value) external {
        rvm.warp(block.timestamp + value);
    }

    /// Return the current block timestamp.
    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }
}
