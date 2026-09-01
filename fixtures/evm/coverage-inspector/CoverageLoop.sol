// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract CoverageLoop {
    function loopN(uint256 n) external {
        for (uint256 i = 0; i < n; i++) {
            uint256 x = i + 1;
        }
    }
}
