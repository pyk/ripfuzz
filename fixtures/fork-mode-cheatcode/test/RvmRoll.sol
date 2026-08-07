// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.roll`
/// cheatcode correctly updates `block.number` in fork mode.
contract RvmRoll {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Roll to `block.number + value`.
    function roll(uint256 value) external {
        rvm.roll(block.number + value);
    }

    /// Return the current block number.
    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }
}
