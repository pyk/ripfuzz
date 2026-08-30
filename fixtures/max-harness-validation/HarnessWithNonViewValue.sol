// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithNonViewValue {
    uint256 internal stored;

    function value() external returns (uint256) {
        return stored;
    }
}
