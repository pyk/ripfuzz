// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract CoverageReportInternalFunctions {
    uint256 public latestValue;

    function add_and_sub(uint256 a, uint256 b) external returns (uint256) {
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
}
