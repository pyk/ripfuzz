// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";

contract TargetContractWithIf is RaptorFuzz {
    uint256 public value;

    constructor() {
        value = 0;
    }

    function runIf(bool condition) external {
        if (condition) {
            value += 1;
        }
    }

    function runIfElse(bool condition) external {
        if (condition) {
            value += 1;
        } else {
            value += 2;
        }
    }

    function runIfElseWithNewline(bool condition) external {
        if (condition) {
            value += 1;
        }

        else {
            value += 2;
        }
    }

    function runNestedIf(bool a, bool b) external {
        if (a) {
            if (b) {
                value += 1;
            }
        } else {
            value += 2;
        }
    }
}
