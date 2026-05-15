// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {ValueGate} from "../src/L2ValueGate.sol";

contract L2ValueGateTest {
    ValueGate public gate;

    function setUp() public {
        gate = new ValueGate();
    }

    function testCatchDragon() public {
        assert(!gate.property_caught());
        gate.unlock(0xBAAAAAAD);
        assert(gate.property_caught());
    }
}
