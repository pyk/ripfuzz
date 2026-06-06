// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";

contract TargetContractBasic is RaptorFuzz {
    uint256 public latestValue;

    constructor() {
        latestValue = 0;
    }

    function addAndSub(uint256 a, uint256 b) external returns (uint256) {
        uint256 result = add(a, b);
        result = sub(result, b);
        return result;
    }

    function add(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a + b;
        return latestValue;
    }

    function sub(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a - b;
        return latestValue;
    }

    // this should have 0 hits
    function div(uint256 a, uint256 b) internal returns (uint256) {
        latestValue = a / b;
        return latestValue;
    }

    function love() external returns (uint256) {
        return add(2, 2);
    }
}
