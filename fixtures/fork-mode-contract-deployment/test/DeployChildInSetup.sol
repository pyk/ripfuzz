// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";
import {BasicContract} from "./BasicContract.sol";

/// @notice Regression fixture for a bug where deploying a child contract
/// inside a setup function in fork mode caused an unnecessary RPC fetch
/// for the child's address.
contract DeployChildInSetup {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    BasicContract public child;

    function setup() external {
        rvm.fork("mock://test", 25_259_523);
        child = new BasicContract();
    }

    function setChildValue(uint256 newValue) external {
        child.setValue(newValue);
    }

    function invariant_child_exists() external view {
        assert(address(child) != address(0));
    }
}
