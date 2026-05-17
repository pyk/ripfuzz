// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeWallet {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    address public derivedAddr;

    function setUp() external {
        // Private key = 1 => known address
        derivedAddr = vm.addr(1);
    }

    function property_addr_correct() external view returns (bool) {
        return derivedAddr == address(0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);
    }
}
