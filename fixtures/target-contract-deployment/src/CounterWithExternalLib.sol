// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MathLibExternal} from "./MathLibExternal.sol";

contract CounterWithExternalLib {
    address public lib;
    uint256 public count;

    constructor(address _lib) {
        lib = _lib;
    }

    function increment() external {
        (bool success, bytes memory result) = lib.delegatecall(
            abi.encodeWithSelector(MathLibExternal.add.selector, count, 1)
        );
        require(success, "library call failed");
        count = abi.decode(result, (uint256));
    }
}
