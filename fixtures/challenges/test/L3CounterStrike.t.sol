// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {CounterStrike} from "../src/L3CounterStrike.sol";

contract L3CounterStrikeTest {
    CounterStrike public strike;

    function setUp() public {
        strike = new CounterStrike();
    }

    function testCatchDragon() public {
        assert(!strike.property_caught());
        for (uint256 i = 0; i < 7; i++) {
            strike.tick();
        }
        strike.claim();
        assert(strike.property_caught());
    }
}
