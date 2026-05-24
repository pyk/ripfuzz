// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Vm} from "./Vm.sol";

contract EmptyChainCheatcodeInSetup {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));

    function setup() external {
        vm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in setup failed");
    }

    function doSomething() external {}
}
