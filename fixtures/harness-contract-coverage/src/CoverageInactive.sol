// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title CoverageInactive
/// @dev A library with two functions used to test that inactive artifacts do not
///      contribute non-executable lines to the coverage report.
library CoverageInactive {
    function usedFunction() internal pure returns (uint256) {
        return 1;
    }

    function unusedFunction() internal pure returns (uint256) {
        return 2;
    }
}
