// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @notice Regression test fixture for startPrank leak bug.
///
/// When vm.startPrank is active, calls made by contracts that were called
/// with the pranked address must NOT see the pranked address as msg.sender.
/// The prank must only apply to calls made by the contract that invoked
/// vm.startPrank.
contract PrankLeakTarget {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    PrankLeakVictim public victim;
    PrankLeakIntermediate public intermediate;

    address constant ALICE = 0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa;

    function setup() external {
        victim = new PrankLeakVictim();
        intermediate = new PrankLeakIntermediate(victim);
    }

    function action() external {
        vm.startPrank(ALICE);
        intermediate.record();
        vm.stopPrank();
    }

    function invariant() external view {
        assert(victim.lastSender() == address(intermediate));
    }
}

/// @notice Intermediate contract that calls PrankLeakVictim.record().
/// Used to verify that startPrank does not leak into sub-calls.
contract PrankLeakIntermediate {
    PrankLeakVictim public victim;

    constructor(PrankLeakVictim _victim) {
        victim = _victim;
    }

    function record() external {
        victim.record();
    }
}

/// @notice Helper contract that records msg.sender.
contract PrankLeakVictim {
    address public lastSender;

    function record() external {
        lastSender = msg.sender;
    }
}
