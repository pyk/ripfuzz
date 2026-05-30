// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MathLib} from "./MathLib.sol";

contract LinkedCounter {
    uint256 public count;

    function increment() external {
        count = MathLib.add(count, 1);
    }
}
