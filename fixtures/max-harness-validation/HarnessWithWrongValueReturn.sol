// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithWrongValueReturn {
    function value() external view returns (uint128) {
        return 0;
    }
}
