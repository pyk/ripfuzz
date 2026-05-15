// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract SetupRevert {
    function setUp() external {
        revert("setUp always reverts");
    }

    function set(uint256 x) external {
        // reachable after setUp
    }
}
