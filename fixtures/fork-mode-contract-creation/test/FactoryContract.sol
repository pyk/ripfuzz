// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BasicContract} from "./BasicContract.sol";

contract FactoryContract {
    BasicContract public child;
    uint256 public value;

    constructor() {
        child = new BasicContract();
        value = 99;
    }

    function setValue(uint256 newValue) external {
        value = newValue;
    }

    function invariant_child_exists() external view {
        assert(address(child) != address(0));
    }
}
