// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptWithSetup {
    bool internal ready;

    function setup() external {
        ready = true;
    }

    function exec() external {
        require(ready, "not ready");
    }
}
