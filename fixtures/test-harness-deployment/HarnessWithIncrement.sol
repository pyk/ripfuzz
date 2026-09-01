// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

contract HarnessWithIncrement {
    uint256 public total;

    function increment(uint256 amount) external {
        total += amount;
    }

    function invariant_uint_roundtrip() external pure {
        uint256 value = 42;
        assert(uint256(uint128(value)) == value);
    }
}
