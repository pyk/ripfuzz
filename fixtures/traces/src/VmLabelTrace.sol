// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "./FoundryVm.sol";

contract VmLabelTrace {
    constructor() {
        Vm vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
        vm.label(0x1111111111111111111111111111111111111111, "ExternalTarget");
    }
}
