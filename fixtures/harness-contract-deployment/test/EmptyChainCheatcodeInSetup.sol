// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";

contract EmptyChainCheatcodeInSetup {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    function setup() external {
        rvm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in setup failed");
    }

    function doSomething() external {}
}
