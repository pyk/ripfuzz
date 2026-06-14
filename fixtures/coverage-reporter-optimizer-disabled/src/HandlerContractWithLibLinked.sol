// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";
import {CounterLinked} from "./CounterLinked.sol";

contract HandlerContractWithLibLinked is RaptorFuzz {
    uint256 public latestValue;
    CounterLinked public counterLinked;

    constructor() {
        counterLinked = new CounterLinked();
    }

    function libLinkedCall(uint256 amount) external returns (uint256) {
        counterLinked.increment(amount);
        latestValue = counterLinked.value();
        return latestValue;
    }
}
