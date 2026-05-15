// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {DeepNestingB} from "./DeepNestingB.sol";

contract DeepNestingA {
    function callB(address b, address c) external {
        DeepNestingB(b).callC(c);
    }
}
