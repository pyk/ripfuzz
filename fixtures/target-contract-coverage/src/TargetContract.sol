// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";

contract TargetContract is RaptorFuzz {
    uint256 public latestValue;

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

    function earlyReturn(uint256 a) external returns (uint256) {
        if (a == 0) {
            return 0;
        }
        latestValue = a;
        return latestValue;
    }

    function inheritanceCall(uint256 a) external returns (uint256) {
        uint256 bounded = bound(a, 10, 100);
        latestValue = bounded;
        return bounded;
    }
}
