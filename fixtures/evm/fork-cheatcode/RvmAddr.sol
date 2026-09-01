// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {RVM} from "./RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.addr`
/// cheatcode correctly derives a local address in fork mode.
contract RvmAddr {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address public actor;

    /// Enter fork mode, then derive an address from private key 1.
    function setup() external {
        rvm.fork("mock://test", 25_259_523);
        actor = rvm.addr(1);
    }

    /// Return the ETH balance of the derived actor.
    function getBalance() external view returns (uint256) {
        return actor.balance;
    }
}
