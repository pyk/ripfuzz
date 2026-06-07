// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BasicContract} from "./BasicContract.sol";

/// @notice Regression fixture for a bug where deploying a child contract
/// inside a setup function in fork mode caused an unnecessary RPC fetch
/// for the child's address.
contract DeployChildInSetup {
    BasicContract public child;

    function setup() external {
        child = new BasicContract();
    }

    function setChildValue(uint256 newValue) external {
        child.setValue(newValue);
    }

    function invariant_child_exists() external view {
        assert(address(child) != address(0));
    }
}
