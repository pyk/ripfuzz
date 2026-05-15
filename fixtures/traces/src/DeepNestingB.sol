// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {DeepNestingC} from "./DeepNestingC.sol";

contract DeepNestingB {
    function callC(address c) external {
        DeepNestingC(c).doSomething();
    }
}
