// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MathLibLinked} from "./MathLibLinked.sol";

contract CounterWithLinkedLib {
    uint256 public count;

    function increment() external {
        count = MathLibLinked.add(count, 1);
    }
}
