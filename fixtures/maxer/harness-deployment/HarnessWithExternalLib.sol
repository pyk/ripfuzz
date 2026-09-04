// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {ExternalMathLib} from "./ExternalMathLib.sol";

contract HarnessWithExternalLib {
    uint256 public total;

    function increment(uint256 amount) external {
        total = ExternalMathLib.add(total, amount);
    }

    function value() external view returns (uint256) {
        return total;
    }
}
