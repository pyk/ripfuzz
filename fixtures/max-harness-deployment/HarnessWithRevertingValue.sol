// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithRevertingValue {
    uint256 internal stored;

    function value() external view returns (uint256) {
        revert("value failed");
    }
}
