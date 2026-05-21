// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {CounterStrike} from "../src/L3CounterStrike.sol";

contract L3CounterStrikeTest {
    CounterStrike public strike;

    function setup() public {
        strike = new CounterStrike();
    }

    function testCatchDragon() public {
        strike.invariant_caught(); // succeeds before dragon
        for (uint256 i = 0; i < 7; i++) {
            strike.tick();
        }
        strike.claim();
        try strike.invariant_caught() {
            revert("invariant should have reverted after dragon");
        } catch {
            // expected revert — dragon caught!
        }
    }
}
