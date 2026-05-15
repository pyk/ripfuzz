// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Helper} from "./Helper.sol";
import {MultiCall} from "./MultiCall.sol";

contract MultiCallTrace {
    constructor() {
        Helper helper = new Helper();
        MultiCall mc = new MultiCall();
        mc.doManyCalls(address(helper));
    }
}
