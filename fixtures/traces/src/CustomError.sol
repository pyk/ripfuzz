// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

error MyCustomError(uint256 code, string message);

contract CustomError {
    function trigger() external pure {
        revert MyCustomError(404, "not found");
    }
}
