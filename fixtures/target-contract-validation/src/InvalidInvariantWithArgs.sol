// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidInvariantWithArgs {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }

    function invariant_check(uint256 x) external view {
        require(x >= 0, "ok");
    }
}
