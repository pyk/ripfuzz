// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LabelCallTrace} from "../src/LabelCallTrace.sol";
import {ExternalTarget} from "../src/ExternalTarget.sol";
import {Vm} from "../src/RaptorVm.sol";

contract LabelCallTest {
    LabelCallTrace public trace;
    ExternalTarget public target;

    function setUp() public {
        target = new ExternalTarget();
        // Pre-deploy ExternalTarget bytecode at the hard-coded address
        // that LabelCallTrace calls.
        Vm vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);
        vm.etch(0x1111111111111111111111111111111111111111, address(target).code);
        vm.label(0x1111111111111111111111111111111111111111, "ExternalTarget");
        trace = new LabelCallTrace();
    }

    function testLabelCall() public view {
        // If setUp completed, LabelCallTrace successfully called
        // 0x1111... which was pre-populated with ExternalTarget code.
        require(address(trace) != address(0), "trace not deployed");
    }
}
