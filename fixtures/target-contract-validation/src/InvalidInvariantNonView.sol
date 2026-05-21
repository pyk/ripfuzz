// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract InvalidInvariantNonView {
    uint256 public value;

    function doSomething() external {
        value = 1;
    }

    function invariant_check() external {
        value = 2;
    }
}
