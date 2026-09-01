// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptWithConstructorArgs {
    uint256 internal initial;

    constructor(uint256 value) {
        initial = value;
    }

    function exec() external {}
}
