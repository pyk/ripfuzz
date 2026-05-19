// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "./RaptorVm.sol";

contract VmLabelTrace {
    constructor() {
        Vm vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);
        vm.label(0x1111111111111111111111111111111111111111, "ExternalTarget");
    }
}
