// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {InternalMathLib} from "./InternalMathLib.sol";

contract HarnessWithInternalLib {
    uint256 public total;

    function increment(uint256 amount) external {
        total = InternalMathLib.add(total, amount);
    }

    function value() external view returns (uint256) {
        return total;
    }
}
