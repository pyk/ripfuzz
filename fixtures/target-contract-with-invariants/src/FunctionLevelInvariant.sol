// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A target contract used to test function-level invariant detection.
///
/// The bug lives inside `three()`: it asserts whenever both `one()` and
/// `two()` have been executed in the same sequence.  The minimal
/// reproducing sequence is exactly `one() -> two() -> three()`.
///
/// Two system-level invariants are also declared so raptor appends them
/// after every call sequence.
contract FunctionLevelInvariant {
    bool public oneCalled;
    bool public twoCalled;

    function one() external {
        oneCalled = true;
    }

    function two() external {
        twoCalled = true;
    }

    function three() external {
        // Function-level invariant: calling three() after both one() and
        // two() must never happen.
        assert(!(oneCalled && twoCalled));
    }

    function invariant_flags_consistent() external view {
        assert(true);
    }

    function invariant_state_valid() external view {
        assert(true);
    }
}
