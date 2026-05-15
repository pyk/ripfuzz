// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {SimpleKnob} from "../src/L1SimpleKnob.sol";

contract L1SimpleKnobTest {
    SimpleKnob public knob;

    function setUp() public {
        knob = new SimpleKnob();
    }

    function testCatchDragon() public {
        assert(!knob.property_caught());
        knob.one();
        knob.two();
        knob.three();
        assert(knob.property_caught());
    }
}
