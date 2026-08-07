// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.deal`
/// cheatcode correctly sets account balances in fork mode.
contract RvmDeal {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address public localAddress;

    /// Derive a local address from private key 1 and store it.
    function setup() external {
        localAddress = rvm.addr(1);
    }

    /// Set the balance of the local address to 1 ether.
    function dealLocalAddress() external {
        rvm.deal(localAddress, 1 ether);
    }

    /// Set the balance of vitalik.eth to 1 ether.
    function dealRemoteAddress() external {
        rvm.deal(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045, 1 ether);
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
