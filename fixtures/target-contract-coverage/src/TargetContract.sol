// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";
import {Counter} from "./Counter.sol";
import {CounterLinked} from "./CounterLinked.sol";
import {ICounter} from "./ICounter.sol";

contract TargetContract is RaptorFuzz {
    uint256 public latestValue;
    Counter public counter;
    CounterLinked public counterLinked;
    ICounter public counterInterface;

    constructor() {
        counter = new Counter();
        counterLinked = new CounterLinked();
        counterInterface = ICounter(address(counter));
    }

    function addAndSub(uint256 a, uint256 b) external returns (uint256) {
        uint256 result = add(a, b);
        result = sub(result, b);
        return result;
    }

    function add(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a + b;
        return latestValue;
    }

    function sub(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a - b;
        return latestValue;
    }

    function earlyReturn(uint256 a) external returns (uint256) {
        if (a == 0) {
            return 0;
        }
        latestValue = a;
        return latestValue;
    }

    function inheritanceCall(uint256 a) external returns (uint256) {
        uint256 bounded = bound(a, 10, 100);
        latestValue = bounded;
        return bounded;
    }

    function libCall(uint256 amount) external returns (uint256) {
        counter.increment(amount);
        latestValue = counter.value();
        return latestValue;
    }

    function libLinkedCall(uint256 amount) external returns (uint256) {
        counterLinked.increment(amount);
        latestValue = counterLinked.value();
        return latestValue;
    }

    function interfaceCall(uint256 amount) external returns (uint256) {
        counterInterface.increment(amount);
        latestValue = counterInterface.value();
        return latestValue;
    }
}
