// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

/// @notice Integration test fixture for verifying that the `vm.chainId`
/// cheatcode correctly updates `block.chainid` in fork mode.
contract VmChainId {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Set chain ID to `value`.
    function setChainId(uint256 value) external {
        vm.chainId(value);
    }

    /// Return the current chain ID.
    function getChainId() external view returns (uint256) {
        return block.chainid;
    }
}