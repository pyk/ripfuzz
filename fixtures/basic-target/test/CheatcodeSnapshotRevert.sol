// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeSnapshotRevert {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));
    uint256 public counter;

    function setup() external {
        counter = 0;
    }

    function increment() external {
        counter++;
    }

    function invariant_counter_never_100() external view {
        assert(counter != 100);
    }
}
