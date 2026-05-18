// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

/**
 * @title ValueGate
 * @notice Pass the right value -> 🐲
 * @dev Level 2: Supply the correct input value to unlock.
 */
contract ValueGate {
    uint256 public property;

    constructor() {
        property = 1 ether;
    }

    function unlock(uint256 key) external {
        if (key == 0xBAAAAAAD) {
            property = 2 ether;
        } else {
            revert(unicode"💀");
        }
    }

    function invariant_caught() external view {
        assert(property != 2 ether);
    }
}
