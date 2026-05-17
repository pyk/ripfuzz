// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeSnapshotRevert {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public counter;

    function setUp() external {
        counter = 0;
    }

    function increment() external {
        counter++;
    }

    function property_counter_never_100() external view returns (bool) {
        return counter != 100;
    }
}
