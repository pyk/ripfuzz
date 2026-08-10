// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness contract with two independent max objectives.
///
/// `max_a()` and `max_b()` are tracked separately, so improving one must not
/// affect the other.
contract MaxMultiple {
    uint256 public a;
    uint256 public b;

    function setA(uint256 x) external {
        a = x;
    }

    function setB(uint256 x) external {
        b = x;
    }

    function max_a() external view returns (uint256) {
        return a;
    }

    function max_b() external view returns (uint256) {
        return b;
    }
}
