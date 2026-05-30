// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {BasicConstructorRevert} from "../src/BasicConstructorRevert.sol";

contract BasicConstructorRevertTest {
    function testDeploy() external {
        new BasicConstructorRevert();
    }
}
