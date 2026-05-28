// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CounterWithLinkedLib} from "../src/CounterWithLinkedLib.sol";

contract EmptyChainDeployLinkedLibInSetup {
    CounterWithLinkedLib public counter;

    function setup() external {
        counter = new CounterWithLinkedLib();
    }

    function doSomething() external {}
}
