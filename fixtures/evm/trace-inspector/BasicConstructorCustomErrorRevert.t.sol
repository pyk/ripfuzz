// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BasicConstructorCustomErrorRevert} from "./BasicConstructorCustomErrorRevert.sol";

contract BasicConstructorCustomErrorRevertTest {
    function testDeploy() external {
        new BasicConstructorCustomErrorRevert();
    }
}
