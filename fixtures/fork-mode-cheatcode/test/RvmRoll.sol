// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.roll`
/// cheatcode correctly updates `block.number` in fork mode.
contract RvmRoll {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Enter fork mode at the pinned mainnet block used by the test mocks.
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
    }

    /// Roll to `block.number + value`.
    function roll(uint256 value) external {
        rvm.roll(block.number + value);
    }

    /// Return the current block number.
    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }
}
