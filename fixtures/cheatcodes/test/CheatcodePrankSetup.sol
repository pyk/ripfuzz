// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";
import {PrankVictim} from "../src/PrankVictim.sol";

contract CheatcodePrankSetup {
    RVM constant rvm = RVM(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    PrankVictim public victim;

    function setup() external {
        victim = new PrankVictim();
        rvm.startPrank(address(0x888));
    }

    function call_override_and_revert() external {
        rvm.startPrank(address(0x999));
        victim.record();
        revert("intentional");
    }

    function call_expect_persisted() external {
        victim.record();
    }

    function setup_start_persisted() external view returns (bool) {
        return victim.lastSender() == address(0x888);
    }
}
