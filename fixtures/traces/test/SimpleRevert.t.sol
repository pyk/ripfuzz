// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/SimpleRevert.sol";

contract SimpleRevertTest {
    function testDeploy() public {
        new SimpleRevert();
    }
}
