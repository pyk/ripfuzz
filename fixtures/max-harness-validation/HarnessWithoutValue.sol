// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithoutValue {
    uint256 internal value;

    function set(uint256 x) external {
        value = x;
    }
}
