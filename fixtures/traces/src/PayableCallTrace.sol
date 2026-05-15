// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {PayableTarget} from "./PayableTarget.sol";

contract PayableCallTrace {
    constructor() payable {
        PayableTarget target = new PayableTarget();
        target.deposit{value: 100}();
    }
}
