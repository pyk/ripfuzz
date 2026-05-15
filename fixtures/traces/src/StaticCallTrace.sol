// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {StaticCallTarget} from "./StaticCallTarget.sol";

contract StaticCallTrace {
    constructor() {
        StaticCallTarget target = new StaticCallTarget();
        target.getStored();
        target.getSum(3, 4);
    }
}
