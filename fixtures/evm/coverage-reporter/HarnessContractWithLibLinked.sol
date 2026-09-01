// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {CounterLinked} from "./CounterLinked.sol";

contract HarnessContractWithLibLinked {
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
