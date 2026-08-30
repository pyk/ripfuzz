// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithRevertingConstructor {
    constructor() {
        revert("nope");
    }
}
