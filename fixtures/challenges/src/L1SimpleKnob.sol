// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

/**
 * @title SimpleKnob
 * @notice one() two() three() -> 🐲
 * @dev Level 1: Call the functions in the correct sequence.
 */
contract SimpleKnob2 {
    uint256 public property;
    uint256 internal _one;
    uint256 internal _two;

    constructor() {
        property = 1 ether;
    }

    function one() external {
        _one = 1 ether;
    }

    function two() external {
        if (_one == 1 ether) {
            _two = 2 ether;
        } else {
            revert(unicode"💀");
        }
    }

    function three() external {
        if (_one == 1 ether && _two == 2 ether) {
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
