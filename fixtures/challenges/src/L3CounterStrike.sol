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

    /// @return true when the dragon is caught.
    function property_caught() external view returns (bool) {
        return property == 3 ether;
    }
}
