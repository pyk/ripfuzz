// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

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
