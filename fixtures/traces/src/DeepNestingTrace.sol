// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {DeepNestingA} from "./DeepNestingA.sol";
import {DeepNestingB} from "./DeepNestingB.sol";
import {DeepNestingC} from "./DeepNestingC.sol";

contract DeepNestingTrace {
    constructor() {
        DeepNestingA a = new DeepNestingA();
        DeepNestingB b = new DeepNestingB();
        DeepNestingC c = new DeepNestingC();
        a.callB(address(b), address(c));
    }
}
