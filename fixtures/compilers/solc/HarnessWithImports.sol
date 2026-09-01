// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {Support} from "./Support.sol";
import {Lib} from "./Lib.sol";

contract HarnessWithImports {
    using Lib for uint256;

    Support public support;
    uint256 public value;

    constructor() {
        support = new Support();
    }

    function set(uint256 x) external {
        value = x.double();
        support.setValue(x);
    }

    function getValue() external view returns (uint256) {
        return value;
    }
}
