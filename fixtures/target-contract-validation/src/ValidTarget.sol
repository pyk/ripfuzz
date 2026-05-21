// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ValidTarget {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }

    function invariant_check() public view {
        require(value >= 0, "ok");
    }
}
