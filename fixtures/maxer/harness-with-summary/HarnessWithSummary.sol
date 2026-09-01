// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Harness with an optional `summary` function for the summary-logs test.
///
/// The summary emits the accumulated total, so the traced summary run must
/// contain both the call and the event.
contract HarnessWithSummary {
    uint256 internal total;

    function deposit(uint256 amount) external {
        total += amount;
    }

    function value() external view returns (uint256) {
        return total;
    }

    event Summarized(uint256 total);

    function summary() external {
        emit Summarized(total);
    }
}
