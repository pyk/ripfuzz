// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract CoverageBranch {
    function branch(bool take) external {
        if (take) {
            uint256 x = 1;
        } else {
            uint256 x = 2;
        }
    }
}
