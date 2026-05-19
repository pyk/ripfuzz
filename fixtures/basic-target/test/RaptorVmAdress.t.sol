// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {console} from "forge-std/console.sol";

contract RaptorVmAddressTest {
    function testRaptorVmAddress() external pure {
        address raptorVm = address(uint160(uint256(keccak256("raptor vm"))));
        console.log("Raptor VM address:", raptorVm);
    }
}
