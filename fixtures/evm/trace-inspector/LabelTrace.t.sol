// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {LabelTrace} from "./LabelTrace.sol";

contract LabelTraceTest {
    function testDeploy() external {
        new LabelTrace();
    }
}
