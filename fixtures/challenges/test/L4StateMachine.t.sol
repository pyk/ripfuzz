// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {StateMachine} from "../src/L4StateMachine.sol";

contract L4StateMachineTest {
    StateMachine public machine;

    function setUp() public {
        machine = new StateMachine();
    }

    function testCatchDragon() public {
        machine.invariant_caught(); // succeeds before dragon
        machine.stepA();
        machine.stepB();
        machine.stepC();
        machine.finish();
        try machine.invariant_caught() {
            revert("invariant should have reverted after dragon");
        } catch {
            // expected revert — dragon caught!
        }
    }
}
