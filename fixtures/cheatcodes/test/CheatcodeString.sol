// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeString {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    string public strUint;
    string public strBool;
    uint256 public parsedUint;
    address public parsedAddr;

    function setUp() external {
        strUint = vm.toString(uint256(123));
        strBool = vm.toString(true);
        parsedUint = vm.parseUint("456");
        parsedAddr = vm.parseAddress("0x71C7656EC7ab88b098defB751B7401B5f6d8976F");
    }

    function property_string_ops_ok() external view returns (bool) {
        return keccak256(bytes(strUint)) == keccak256(bytes("123"))
            && keccak256(bytes(strBool)) == keccak256(bytes("true"))
            && parsedUint == 456
            && parsedAddr == address(0x71C7656EC7ab88b098defB751B7401B5f6d8976F);
    }
}
