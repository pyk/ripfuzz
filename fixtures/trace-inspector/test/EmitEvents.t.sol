// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {EmitEvents} from "../src/EmitEvents.sol";

contract EmitEventsTest {
    function testDeploy() external {
        new EmitEvents();
    }
}
