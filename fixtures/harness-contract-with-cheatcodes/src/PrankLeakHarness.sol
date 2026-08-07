// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @notice Regression test fixture for startPrank leak bug.
///
/// When rvm.startPrank is active, calls made by contracts that were called
/// with the pranked address must NOT see the pranked address as msg.sender.
/// The prank must only apply to calls made by the contract that invoked
/// rvm.startPrank.
contract PrankLeakHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    PrankLeakVictim public victim;
    PrankLeakIntermediate public intermediate;

    address constant ALICE = 0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa;

    function setup() external {
        victim = new PrankLeakVictim();
        intermediate = new PrankLeakIntermediate(victim);
    }

    function action() external {
        rvm.startPrank(ALICE);
        intermediate.record();
        rvm.stopPrank();
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
