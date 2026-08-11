// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A max-mode harness whose handler always reverts.
///
/// Every generated sequence fails on its first call, so with
/// `--fail-on-revert` the campaign must stop at the first failure and report
/// it instead of running to completion with zero failed assertions.
contract MaxFailOnRevert {
    uint256 internal value;

    function revert_always() external {
        revert("skip precondition");
    }

    function max_value() external view returns (uint256) {
        return value;
    }
}
