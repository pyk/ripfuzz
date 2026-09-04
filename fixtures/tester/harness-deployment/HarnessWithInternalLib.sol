// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {InternalMathLib} from "./InternalMathLib.sol";

contract HarnessWithInternalLib {
    uint256 public total;

    function increment(uint256 amount) external {
        total = InternalMathLib.add(total, amount);
    }

    function invariant_uint_roundtrip() external pure {
        uint256 value = 42;
        assert(uint256(uint128(value)) == value);
    }
}
