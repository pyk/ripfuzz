// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAssertionsSetupFail {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    function setUp() external {
        vm.deny(true, "setUp must abort");
    }

    function call_never_reached() external {
        // If setup aborted, this function is never callable.
    }
}
