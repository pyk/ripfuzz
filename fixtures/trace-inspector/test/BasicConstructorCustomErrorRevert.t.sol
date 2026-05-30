// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {BasicConstructorCustomErrorRevert} from "../src/BasicConstructorCustomErrorRevert.sol";

contract BasicConstructorCustomErrorRevertTest {
    function testDeploy() external {
        new BasicConstructorCustomErrorRevert();
    }
}
