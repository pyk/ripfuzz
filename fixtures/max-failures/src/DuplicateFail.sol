// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness where the same bug can be triggered with many different argument
/// values. All failing runs share the same function sequence and failing
/// position, so they must deduplicate into a single failed assertion.
contract DuplicateFail {
    function trigger(uint256 x) external {
        assert(x != 0);
    }
}
