// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

contract PayableConstructorHarness {
    uint256 public received;

    constructor() payable {
        received = msg.value;
    }

    function invariant_received_nonzero() external view {
        assert(received > 0);
    }
}
