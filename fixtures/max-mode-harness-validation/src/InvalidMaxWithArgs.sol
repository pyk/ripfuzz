// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract InvalidMaxWithArgs {
    function max_value(uint256 x) external pure returns (uint256) {
        return x;
    }
}
