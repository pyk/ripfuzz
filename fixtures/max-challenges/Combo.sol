// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: hard
///
/// The reward of 1000 is only paid when `open`, `grab`, and `claim` run in
/// that exact order. `noise` is a decoy that does nothing.
contract Combo {
    uint256 internal step;
    uint256 internal reward;

    function open() external {
        require(step == 0);
        step = 1;
    }

    function grab() external {
        require(step == 1);
        step = 2;
    }

    function claim() external {
        require(step == 2);
        reward = 1000;
    }

    function noise() external {}

    function value() external view returns (uint256) {
        return reward;
    }
}
