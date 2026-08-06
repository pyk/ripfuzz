// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidNoHandlers {
    uint256 private value;

    function setup() external {
        value = 0;
    }

    function invariant_check() external view {
        require(value >= 0, "ok");
    }
}
