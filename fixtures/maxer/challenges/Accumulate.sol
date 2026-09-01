// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: easy
///
/// The highest value is `type(uint256).max`, reached by depositing
/// `type(uint256).max` while the total is still zero.
contract Accumulate {
    uint256 internal total;

    function deposit(uint256 amount) external {
        total += amount;
    }

    function value() external view returns (uint256) {
        return total;
    }
}
