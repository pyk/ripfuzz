// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MathLib} from "./MathLib.sol";

contract Counter {
    uint256 public value;

    function increment(uint256 amount) external {
        value = MathLib.add(value, amount);
    }
}
