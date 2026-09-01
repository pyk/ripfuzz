// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {ICounter} from "./ICounter.sol";
import {Counter} from "./Counter.sol";

contract HarnessContractWithInterface {
    uint256 public latestValue;
    ICounter public counterInterface;

    constructor() {
        counterInterface = ICounter(address(new Counter()));
    }

    function interfaceCall(uint256 amount) external returns (uint256) {
        counterInterface.increment(amount);
        latestValue = counterInterface.value();
        return latestValue;
    }
}
