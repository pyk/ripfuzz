// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LabelCallTrace} from "../src/LabelCallTrace.sol";
import {ExternalTarget} from "../src/ExternalTarget.sol";
import {Vm} from "../src/FoundryVm.sol";

contract LabelCallTest {
    LabelCallTrace public trace;
    ExternalTarget public target;

    function setUp() public {
        target = new ExternalTarget();
        // Pre-deploy ExternalTarget bytecode at the hard-coded address
        // that LabelCallTrace calls.
        Vm vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
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
