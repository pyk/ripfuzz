// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {EmitEvents} from "./EmitEvents.sol";

contract EmitEventsTest {
    function testDeploy() external {
        new EmitEvents();
    }
}
