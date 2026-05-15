// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Helper} from "../src/Helper.sol";

contract ComplexConstructorRevert {
    constructor() {
        Helper helper = new Helper();
        helper.setValue(200);
        revert("constructor always reverts");
    }
}
