// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LabelTrace} from "../src/LabelTrace.sol";

contract LabelTraceTest {
    function testDeploy() external {
        new LabelTrace();
    }
}
