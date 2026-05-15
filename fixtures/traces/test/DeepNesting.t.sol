// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/DeepNestingA.sol";
import "../src/DeepNestingB.sol";
import "../src/DeepNestingC.sol";

contract DeepNestingTest {
    function testDeepNesting() public {
        DeepNestingA a = new DeepNestingA();
        DeepNestingB b = new DeepNestingB();
        DeepNestingC c = new DeepNestingC();
        a.callB(address(b), address(c));
    }
}
