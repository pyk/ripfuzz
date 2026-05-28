// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MathLib} from "./MathLib.sol";

contract CounterWithLib {
    using MathLib for uint256;

    uint256 public count;

    function increment() external {
        count = count.add(1);
    }
}
