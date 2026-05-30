// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {BasicConstructorAssertionFailed} from "../src/BasicConstructorAssertionFailed.sol";

contract BasicConstructorAssertionFailedTest {
    function testDeploy() external {
        new BasicConstructorAssertionFailed();
    }
}
