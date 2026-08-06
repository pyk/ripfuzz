// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RipFuzz} from "./RipFuzz.sol";

contract HarnessContractWithLoop is RipFuzz {
    uint256 public value;

    constructor() {
        for (uint256 i = 0; i < 3; i++) {
            value += i;
        }
    }

    function setup() external {
        for (uint256 i = 0; i < 2; i++) {
            value += i + 1;
        }
    }

    function runLoop(uint256 count) external {
        uint256 bounded = bound(count, 1, 5);
        for (uint256 i = 0; i < bounded; i++) {
            value += i + 1;
        }
    }

    function runNestedLoop(uint256 outer, uint256 inner) external {
        uint256 boundedOuter = bound(outer, 1, 3);
        uint256 boundedInner = bound(inner, 1, 3);
        for (uint256 i = 0; i < boundedOuter; i++) {
            for (uint256 j = 0; j < boundedInner; j++) {
                value += i + j + 1;
            }
        }
    }
}
