// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Vm} from "../src/Vm.sol";

contract EmptyChainCheatcodeInConstructor {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));

    constructor() {
        vm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in constructor failed");
    }

    function doSomething() external {}
}
