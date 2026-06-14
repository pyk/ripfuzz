// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Vm} from "../src/Vm.sol";

contract EmptyChainCheatcodeInConstructor {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    constructor() {
        vm.warp(1234567890);
        require(block.timestamp == 1234567890, "warp in constructor failed");
    }

    function doSomething() external {}
}
