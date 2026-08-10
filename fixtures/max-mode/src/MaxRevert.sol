// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract whose max objective reverts until state is set.
///
/// Reverted max calls must decode as the minimum score (`0`), so only a
/// sequence that calls `set()` can improve the maximum.
contract MaxRevert {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }

    function max_value() external view returns (uint256) {
        require(value != 0, "value is zero");
        return value;
    }
}
