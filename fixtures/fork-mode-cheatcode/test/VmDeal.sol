// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

/// @notice Integration test fixture for verifying that the `vm.deal`
/// cheatcode correctly sets account balances in fork mode.
contract VmDeal {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address public localAddress;

    /// Derive a local address from private key 1 and store it.
    function setup() external {
        localAddress = vm.addr(1);
    }

    /// Set the balance of the local address to 1 ether.
    function dealLocalAddress() external {
        vm.deal(localAddress, 1 ether);
    }

    /// Set the balance of vitalik.eth to 1 ether.
    function dealRemoteAddress() external {
        vm.deal(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045, 1 ether);
    }

    /// Return the ETH balance of the local address.
    function getLocalBalance() external view returns (uint256) {
        return localAddress.balance;
    }

    /// Return the ETH balance of vitalik.eth.
    function getRemoteBalance() external view returns (uint256) {
        return address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045).balance;
    }
}
