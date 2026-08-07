// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.warp`
/// cheatcode correctly updates `block.timestamp` in fork mode.
contract RvmWarp {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Warp to `block.timestamp + value`.
    function warp(uint256 value) external {
        rvm.warp(block.timestamp + value);
    }

    /// Return the current block timestamp.
    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }
}
