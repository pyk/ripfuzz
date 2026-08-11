// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A max-mode harness whose handler always reverts.
///
/// Every generated sequence reverts on its first call, so with
/// `--stop-on-revert` the campaign must stop at the first revert and dump the
/// whole trace into the log instead of running to completion.
contract MaxStopOnRevert {
    uint256 internal value;

    function revert_always() external {
        revert("skip precondition");
    }

    function max_value() external view returns (uint256) {
        return value;
    }
}
