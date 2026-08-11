// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// An invariant-mode harness whose handler always reverts.
///
/// With `--stop-on-revert` the campaign must stop at the first revert and
/// dump the whole trace into the log instead of running to completion.
contract InvariantStopOnRevert {
    uint256 internal constant MAX = 100;
    uint256 internal value;

    function revert_always() external {
        revert("skip precondition");
    }

    function invariant_value_lt_100() external view {
        assert(value < MAX);
    }
}
