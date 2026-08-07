// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.chainId`
/// cheatcode correctly updates `block.chainid` in fork mode.
contract RvmChainId {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Enter fork mode at the pinned mainnet block used by the test mocks.
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
    }

    /// Set chain ID to `value`.
    function setChainId(uint256 value) external {
        rvm.chainId(value);
    }

    /// Return the current chain ID.
    function getChainId() external view returns (uint256) {
        return block.chainid;
    }
}
