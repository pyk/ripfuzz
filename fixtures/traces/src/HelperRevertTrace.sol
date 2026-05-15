// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Helper} from "./Helper.sol";

contract HelperRevertTrace {
    constructor() {
        Helper helper = new Helper();
        helper.doRevert();
    }
}
