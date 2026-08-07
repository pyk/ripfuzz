// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RipFuzz} from "./RipFuzz.sol";
import {ICounter} from "./ICounter.sol";
import {RVM} from "./RVM.sol";

contract HarnessContractWithInterface is RipFuzz {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);
    uint256 public latestValue;
    ICounter public counterInterface;

    constructor() {
        bytes memory code = rvm.getCode("src/Counter.sol:Counter");
        address counter;
        assembly {
            counter := create(0, add(code, 0x20), mload(code))
        }
        require(counter != address(0), "deployment failed");
        counterInterface = ICounter(counter);
    }

    function interfaceCall(uint256 amount) external returns (uint256) {
        counterInterface.increment(amount);
        latestValue = counterInterface.value();
        return latestValue;
    }
}
