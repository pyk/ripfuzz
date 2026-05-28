// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CounterWithExternalLib} from "./CounterWithExternalLib.sol";

contract EmptyChainDeployContractWithExternalLibInSetup {
    address public lib;
    CounterWithExternalLib public counter;

    function setup() external {
        counter = new CounterWithExternalLib(address(lib));
    }

    function setLib(address _lib) external {
        lib = _lib;
    }

    function doSomething() external {}
}
