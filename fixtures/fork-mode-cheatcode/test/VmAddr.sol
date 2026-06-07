// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

/// @notice Integration test fixture for verifying that the `vm.addr`
/// cheatcode correctly derives a local address in fork mode.
contract VmAddr {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address public actor;

    /// Derive an address from private key 1 and store it.
    function setup() external {
        actor = vm.addr(1);
    }

    /// Return the ETH balance of the derived actor.
    function getBalance() external view returns (uint256) {
        return actor.balance;
    }
}