// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithInvariantFunction {
    uint256 internal stored;

    function value() external view returns (uint256) {
        return stored;
    }

    function invariant_value_is_zero() external view {
        assert(stored == 0);
    }
}
