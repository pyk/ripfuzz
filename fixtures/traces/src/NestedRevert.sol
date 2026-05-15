// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Helper} from "./Helper.sol";

contract NestedRevert {
    constructor() {
        Helper helper = new Helper();
        helper.setValue(200);
        revert("nested revert reason");
    }
}
