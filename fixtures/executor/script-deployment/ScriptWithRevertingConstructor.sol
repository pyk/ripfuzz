// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptWithRevertingConstructor {
    constructor() {
        revert("constructor failed");
    }

    function exec() external {}
}
