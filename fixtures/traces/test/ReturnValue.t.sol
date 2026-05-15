// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/ReturnValue.sol";

contract ReturnValueTest {
    function testReturnValue() public {
        ReturnValue target = new ReturnValue();
        target.getBool();
        target.getString();
        target.getAddress();
        target.add(10, 20);
    }
}
