// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract SetupRevert {
    function setup() external {
        revert("setup always reverts");
    }

    function set(uint256 x) external {
        // reachable after setup
    }
}
