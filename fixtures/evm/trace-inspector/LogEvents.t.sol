// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {LogEvents} from "./LogEvents.sol";

contract LogEventsTest {
    function testDeploy() external {
        new LogEvents();
    }
}
