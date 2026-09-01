// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BasicConstructorComplexRevert} from "./BasicConstructorComplexRevert.sol";

contract BasicConstructorComplexRevertTest {
    function testDeploy() external {
        new BasicConstructorComplexRevert{value: 10000}();
    }
}
