// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract InvalidMaxNonView {
    uint256 internal value;

    function max_value() external returns (uint256) {
        value = 0;
        return 0;
    }
}
