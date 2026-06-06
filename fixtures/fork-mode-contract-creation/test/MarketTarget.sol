// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MiddleContract} from "./MiddleContract.sol";

contract MarketTarget {
    MiddleContract public middle;
    uint256 public touched;

    constructor() {
        touched = 0;
    }

    function setup() external {
        middle = new MiddleContract();
    }

    function touchMarket() external {
        middle.createLeaf();
        touched += 1;
    }

    function invariant_middle_exists() external view {
        assert(address(middle) != address(0));
    }
}
