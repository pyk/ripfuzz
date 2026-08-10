// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness with exactly one bug: arming the contract then checking the
/// invariant must fail. The minimal reproducing sequence is
/// `arm() -> invariant_never_armed()`.
contract SingleFail {
    bool public armed;

    function arm() external {
        armed = true;
    }

    function invariant_never_armed() external view {
        assert(!armed);
    }
}
