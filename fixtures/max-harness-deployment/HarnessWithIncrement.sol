// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithIncrement {
    uint256 public count;

    function increment() external {
        count += 1;
    }
}
