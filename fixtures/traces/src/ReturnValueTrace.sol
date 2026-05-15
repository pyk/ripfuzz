// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {ReturnValue} from "./ReturnValue.sol";

contract ReturnValueTrace {
    constructor() {
        ReturnValue target = new ReturnValue();
        target.getBool();
        target.getString();
        target.getAddress();
        target.add(10, 20);
    }
}
