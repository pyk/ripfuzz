// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAccount {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public storedSlot;

    function setUp() external {
        // deal + setNonce
        vm.deal(address(this), 5 ether);
        vm.setNonce(address(this), 7);

        // store + load roundtrip
        vm.store(address(this), bytes32(uint256(1)), bytes32(uint256(0xCAFE)));

        // etch a tiny runtime contract (PUSH1 0x01 PUSH1 0x00 MSTORE RETURN)
        bytes memory code = hex"6001600052602060006000f3";
        vm.etch(address(0xBEEF), code);
    }

    function call() external {
        storedSlot = uint256(vm.load(address(this), bytes32(uint256(1))));
    }

    function property_account_correct() external view returns (bool) {
        return address(this).balance == 5 ether
            && storedSlot == 0xCAFE;
    }

    function property_etched_code_runs() external view returns (bool) {
        // extcodesize of the etched address should be non-zero
        uint256 size;
        address target = address(0xBEEF);
        assembly {
            size := extcodesize(target)
        }
        return size > 0;
    }
}
