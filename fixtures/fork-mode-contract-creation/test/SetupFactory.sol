// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BasicContract} from "./BasicContract.sol";

contract SetupFactory {
    BasicContract public child;
    uint256 public value;

    constructor() {
        value = 1;
    }

    function setup() external {
        child = new BasicContract();
        value = 99;
    }

    function invariant_child_exists() external view {
        assert(address(child) != address(0));
    }
}
