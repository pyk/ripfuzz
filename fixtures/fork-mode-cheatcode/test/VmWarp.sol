// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

/// @notice Integration test fixture for verifying that the `vm.warp`
/// cheatcode correctly updates `block.timestamp` in fork mode.
contract VmWarp {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Warp to `block.timestamp + value`.
    function warp(uint256 value) external {
        vm.warp(block.timestamp + value);
    }

    /// Return the current block timestamp.
    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }
}
