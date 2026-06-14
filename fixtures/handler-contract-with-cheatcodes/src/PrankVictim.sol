// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Helper contract that records `msg.sender` and `tx.origin` so
/// prank integration tests can observe caller spoofing externally.
contract PrankVictim {
    address public lastSender;
    address public lastOrigin;

    constructor() {
        lastSender = msg.sender;
        lastOrigin = tx.origin;
    }

    function record() external {
        lastSender = msg.sender;
        lastOrigin = tx.origin;
    }

    function nestedRecord(PrankVictim inner) external {
        lastSender = msg.sender;
        lastOrigin = tx.origin;
        inner.record();
    }
}
