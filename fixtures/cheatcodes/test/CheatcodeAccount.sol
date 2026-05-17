// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAccount {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public storedSlot;

    function setUp() external {
        // deal
        vm.deal(address(this), 5 ether);

        // store + load roundtrip
        vm.store(address(this), bytes32(uint256(1)), bytes32(uint256(0xCAFE)));
    }

    function call() external {
        storedSlot = uint256(vm.load(address(this), bytes32(uint256(1))));
    }

    function property_account_correct() external view returns (bool) {
        return address(this).balance == 5 ether
            && storedSlot == 0xCAFE;
    }
}
