// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/StaticCallTarget.sol";

contract StaticCallTest {
    function testStaticCall() public {
        StaticCallTarget target = new StaticCallTarget();
        target.getStored();
        target.getSum(3, 4);
    }
}
