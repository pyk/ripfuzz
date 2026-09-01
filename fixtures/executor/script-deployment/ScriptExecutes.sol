// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract ScriptExecutes {
    event SetupRan();
    event ExecRan(string message);

    bool internal ready;

    function setup() external {
        ready = true;
        emit SetupRan();
    }

    function exec() external {
        require(ready, "setup did not run");
        emit ExecRan("script executed");
    }
}
