// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeSnapshotRevert {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));
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
