// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract used to test basic max-mode fuzzing.
///
/// The max objective is `max_value()`: it returns the stored value, so the
/// fuzzer must call `set()` with a large input to improve the maximum.
contract MaxBasic {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function max_value() external view returns (uint256) {
        return value;
    }
}
