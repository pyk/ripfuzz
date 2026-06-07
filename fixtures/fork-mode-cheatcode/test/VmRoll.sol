// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

/// @notice Integration test fixture for verifying that the `vm.roll`
/// cheatcode correctly updates `block.number` in fork mode.
contract VmRoll {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Roll to `block.number + value`.
    function roll(uint256 value) external {
        vm.roll(block.number + value);
    }

    /// Return the current block number.
    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }
}
