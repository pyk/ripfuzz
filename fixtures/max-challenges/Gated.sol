// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: medium
///
/// The highest value is `type(uint256).max`, reached by calling `enter`
/// before a deposit of `type(uint256).max`. Deposits before `enter` revert.
contract Gated {
    bool internal entered;
    uint256 internal total;

    function enter() external {
        entered = true;
    }

    function deposit(uint256 amount) external {
        require(entered);
        unchecked {
            total += amount;
        }
    }

    function value() external view returns (uint256) {
        return total;
    }
}
