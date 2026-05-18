// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

/**
 * @title CounterStrike
 * @notice Count to exactly 7 -> 🐲
 * @dev Level 3: Call tick() exactly seven times, no more, no less.
 */
contract CounterStrike {
    uint256 public property;
    uint256 internal _ticks;

    constructor() {
        property = 1 ether;
    }

    function tick() external {
        _ticks += 1;
    }

    function claim() external {
        if (_ticks == 7) {
            property = 3 ether;
        } else {
            revert(unicode"💀");
        }
    }

    function invariant_caught() external view {
        assert(property != 3 ether);
    }
}
