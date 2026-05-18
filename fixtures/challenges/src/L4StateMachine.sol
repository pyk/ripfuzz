// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

/**
 * @title StateMachine
 * @notice Start → A → B → C → 🐲
 * @dev Level 4: Navigate a state machine. Wrong moves reset progress.
 */
contract StateMachine {
    uint256 public property;
    uint256 internal _state; // 0 = idle, 1 = A, 2 = B, 3 = C

    constructor() {
        property = 1 ether;
    }

    function stepA() external {
        if (_state == 0) {
            _state = 1;
        } else {
            _state = 0; // reset
        }
    }

    function stepB() external {
        if (_state == 1) {
            _state = 2;
        } else {
            _state = 0; // reset
        }
    }

    function stepC() external {
        if (_state == 2) {
            _state = 3;
        } else {
            _state = 0; // reset
        }
    }

    function finish() external {
        if (_state == 3) {
            property = 4 ether;
        } else {
            revert(unicode"💀");
        }
    }

    function invariant_caught() external view {
        assert(property != 4 ether);
    }
}
