// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";

contract EmptyChainCheatcodeInSetup {
    RVM constant rvm = RVM(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    function setup() external {
        rvm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in setup failed");
    }

    function doSomething() external {}
}
