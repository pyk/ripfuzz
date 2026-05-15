// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Helper} from "./Helper.sol";

contract MultiCall {
    function doManyCalls(address helper) external {
        Helper(helper).setValue(150);
        Helper(helper).getValue();
        Helper(helper).setValue(250);
    }
}
