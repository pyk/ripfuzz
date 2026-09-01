// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {RVM} from "./RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.load`
/// cheatcode correctly reads storage from local and remote contracts
/// in fork mode.
contract RvmLoad {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    LocalContract public localContract;

    /// Enter fork mode, then deploy a LocalContract whose storage slot 0
    /// holds a known value (42).
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
        localContract = new LocalContract();
    }

    /// Use rvm.load to read storage slot 0 from the local contract and
    /// assert it equals 42.
    function loadLocalContract() external {
        bytes32 value = rvm.load(address(localContract), bytes32(uint256(0)));
        require(value == bytes32(uint256(42)), "local load mismatch");
    }

    /// Use rvm.load to read WETH decimals from storage slot 2 and
    /// assert it equals 18 (0x12).
    function loadRemoteContract() external {
        bytes32 decimals = rvm.load(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2, bytes32(uint256(2)));
        require(decimals == bytes32(uint256(18)), "remote load mismatch");
    }
}

/// Simple contract with a known storage value at slot 0.
contract LocalContract {
    uint256 public value = 42;
}
