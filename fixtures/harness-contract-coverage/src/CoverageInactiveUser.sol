// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CoverageInactive} from "./CoverageInactive.sol";

/// @title CoverageInactiveUser
/// @dev A contract that uses CoverageInactive.usedFunction.
contract CoverageInactiveUser {
    function callUsed() external pure returns (uint256) {
        return CoverageInactive.usedFunction();
    }
}
