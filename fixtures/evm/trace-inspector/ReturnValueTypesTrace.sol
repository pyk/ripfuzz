// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {ReturnValueTypes} from "./ReturnValueTypes.sol";

contract ReturnValueTypesTrace {
    constructor() {
        ReturnValueTypes rv = new ReturnValueTypes();
        rv.returnBool();
        rv.returnUint256();
        rv.returnInt256();
        rv.returnAddress();
        rv.returnBytes32();
        rv.returnString();
        rv.returnBytes();
        rv.returnStruct();
        rv.returnArray();
        rv.returnFixedArray();
        rv.returnMultiple();
        rv.returnNestedStruct();
        revert("return value types trace");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
