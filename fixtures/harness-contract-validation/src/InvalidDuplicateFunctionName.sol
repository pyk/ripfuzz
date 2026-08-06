// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidDuplicateFunctionName {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }

    function doSomething(uint256 x) external {
        value = x;
    }

    function invariant_check() external view {
        require(value >= 0, "ok");
    }
}
