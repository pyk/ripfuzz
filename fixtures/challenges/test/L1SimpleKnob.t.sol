// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {SimpleKnob} from "../src/L1SimpleKnob.sol";

contract L1SimpleKnobTest {
    SimpleKnob public knob;

    function setup() public {
        knob = new SimpleKnob();
    }

    function testCatchDragon() public {
        knob.invariant_caught(); // succeeds before dragon
        knob.one();
        knob.two();
        knob.three();
        try knob.invariant_caught() {
            revert("invariant should have reverted after dragon");
        } catch {
            // expected revert — dragon caught!
        }
    }
}
