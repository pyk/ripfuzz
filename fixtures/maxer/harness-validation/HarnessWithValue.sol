// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithValue {
    uint256 internal stored;

    function set(uint256 x) external {
        stored = x;
    }

    function value() external view returns (uint256) {
        return stored;
    }
}
