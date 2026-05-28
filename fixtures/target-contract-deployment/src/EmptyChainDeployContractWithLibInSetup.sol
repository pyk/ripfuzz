// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CounterWithLib} from "./CounterWithLib.sol";

contract EmptyChainDeployContractWithLibInSetup {
    CounterWithLib public counter;

    function setup() external {
        counter = new CounterWithLib();
    }

    function doSomething() external {}
}
