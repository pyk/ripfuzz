// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {RVM} from "./RVM.sol";

/// @notice Regression fixture for a bug where deploying a basic contract
/// in fork mode caused an unnecessary RPC fetch for the newly created address.
contract BasicContract {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    uint256 public value;

    constructor() {
        rvm.fork("mock://test", 25_259_523);
        value = 42;
    }

    function setValue(uint256 newValue) external {
        value = newValue;
    }

    function invariant_value_not_zero() external view {
        assert(value > 0);
    }
}
