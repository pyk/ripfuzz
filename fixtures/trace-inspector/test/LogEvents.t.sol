// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LogEvents} from "../src/LogEvents.sol";

contract LogEventsTest {
    function testDeploy() external {
        new LogEvents();
    }
}
