// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BasicConstructorAssertionFailed} from "./BasicConstructorAssertionFailed.sol";

contract BasicConstructorAssertionFailedTest {
    function testDeploy() external {
        new BasicConstructorAssertionFailed();
    }
}
