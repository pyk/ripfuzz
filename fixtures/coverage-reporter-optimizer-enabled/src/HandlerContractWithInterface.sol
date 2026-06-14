// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";
import {ICounter} from "./ICounter.sol";
import {Vm} from "./Vm.sol";

contract HandlerContractWithInterface is RaptorFuzz {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    uint256 public latestValue;
    ICounter public counterInterface;

    constructor() {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
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
