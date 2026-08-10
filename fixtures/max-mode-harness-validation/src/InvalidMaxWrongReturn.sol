// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract InvalidMaxWrongReturn {
    function max_value() external pure returns (bool) {
        return true;
    }
}
