// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RipFuzz} from "./RipFuzz.sol";
import {Counter} from "./Counter.sol";

contract HandlerContractWithLib is RipFuzz {
    uint256 public latestValue;
    Counter public counter;

    constructor() {
        counter = new Counter();
    }

    function libCall(uint256 amount) external returns (uint256) {
        counter.increment(amount);
        latestValue = counter.value();
        return latestValue;
    }
}
