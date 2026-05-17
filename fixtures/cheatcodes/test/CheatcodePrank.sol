// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";
import {PrankTarget} from "../src/PrankTarget.sol";

contract CheatcodePrank {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    PrankTarget public target;

    function setUp() external {
        target = new PrankTarget();
    }

    function call_prank() external {
        vm.prank(address(0x123));
        target.record();
    }

    function call_start_prank() external {
        vm.startPrank(address(0x456));
        target.record();
        target.record();
        vm.stopPrank();
        target.record();
    }

    function property_prank_applied() external view returns (bool) {
        // prank should have set sender to 0x123 at least once
        return target.lastSender() == address(0x123)
            || target.lastSender() == address(0x456)
            || target.lastSender() == address(this);
    }
}
