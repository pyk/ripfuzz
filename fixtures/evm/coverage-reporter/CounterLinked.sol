// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {MathLibLinked} from "./MathLibLinked.sol";

contract CounterLinked {
    uint256 public value;

    function increment(uint256 amount) external {
        value = MathLibLinked.add(value, amount);
    }
}
