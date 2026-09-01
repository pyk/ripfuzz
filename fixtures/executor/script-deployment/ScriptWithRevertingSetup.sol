// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptWithRevertingSetup {
    function setup() external {
        revert("setup failed");
    }

    function exec() external {}
}
