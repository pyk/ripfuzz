// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {BasicConstructorComplexRevert} from "../src/BasicConstructorComplexRevert.sol";

contract BasicConstructorComplexRevertTest {
    function testDeploy() external {
        new BasicConstructorComplexRevert{value: 10000}();
    }
}
