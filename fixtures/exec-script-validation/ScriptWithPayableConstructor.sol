// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptWithPayableConstructor {
    constructor() payable {}

    function exec() external {}
}
