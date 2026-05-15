// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {CustomError} from "./CustomError.sol";

contract CustomErrorTrace {
    constructor() {
        CustomError target = new CustomError();
        target.trigger();
    }
}
