// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {DelegateCallTarget} from "./DelegateCallTarget.sol";

contract DelegateCallTrace {
    uint256 public value;

    constructor() {
        DelegateCallTarget target = new DelegateCallTarget();
        (bool ok, ) = address(target).delegatecall(
            abi.encodeWithSelector(DelegateCallTarget.setValue.selector, 99)
        );
        require(ok);
    }
}
