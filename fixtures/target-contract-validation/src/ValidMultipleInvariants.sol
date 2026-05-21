// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract ValidMultipleInvariants {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }

    function invariant_a() external view {
        require(value >= 0, "ok");
    }

    function invariant_b() external pure {
        require(true, "ok");
    }
}
