// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BasicConstructorRevert} from "./BasicConstructorRevert.sol";

contract BasicConstructorRevertTest {
    function testDeploy() external {
        new BasicConstructorRevert();
    }
}
