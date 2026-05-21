// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {ValueGate} from "../src/L2ValueGate.sol";

contract L2ValueGateTest {
    ValueGate public gate;

    function setup() public {
        gate = new ValueGate();
    }

    function testCatchDragon() public {
        gate.invariant_caught(); // succeeds before dragon
        gate.unlock(0xBAAAAAAD);
        try gate.invariant_caught() {
            revert("invariant should have reverted after dragon");
        } catch {
            // expected revert — dragon caught!
        }
    }
}
