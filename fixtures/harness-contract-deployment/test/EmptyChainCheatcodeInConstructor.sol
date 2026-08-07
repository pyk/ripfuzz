// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";

contract EmptyChainCheatcodeInConstructor {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    constructor() {
        rvm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in constructor failed");
    }

    function doSomething() external {}
}
