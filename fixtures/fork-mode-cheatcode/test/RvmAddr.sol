// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.addr`
/// cheatcode correctly derives a local address in fork mode.
contract RvmAddr {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address public actor;

    /// Derive an address from private key 1 and store it.
    function setup() external {
        actor = rvm.addr(1);
    }

    /// Return the ETH balance of the derived actor.
    function getBalance() external view returns (uint256) {
        return actor.balance;
    }
}