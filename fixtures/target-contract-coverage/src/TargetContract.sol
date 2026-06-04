// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";
import {Counter} from "./Counter.sol";
import {ICounter} from "./ICounter.sol";

contract TargetContract is RaptorFuzz {
    uint256 public latestValue;
    Counter public counter;
    ICounter public counterInterface;

    constructor() {
        counter = new Counter();
        counterInterface = ICounter(address(counter));
    }

    function inheritanceCall(uint256 a) external returns (uint256) {
        uint256 bounded = bound(a, 10, 100);
        latestValue = bounded;
        return bounded;
    }

    function interfaceCall(uint256 amount) external returns (uint256) {
        counterInterface.increment(amount);
        latestValue = counterInterface.value();
        return latestValue;
    }
}
