// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/NestedRevert.sol";

contract NestedRevertTest {
    function testDeploy() public {
        new NestedRevert();
    }
}
