// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {VmLabelTrace} from "../src/VmLabelTrace.sol";
import {ExternalTarget} from "../src/ExternalTarget.sol";

contract VmLabelTest {
    VmLabelTrace public trace;
    ExternalTarget public target;

    function setUp() public {
        target = new ExternalTarget();
        trace = new VmLabelTrace();
    }

    function testVmLabel() public view {
        require(target.value() == 0, "target should be fresh");
    }
}
