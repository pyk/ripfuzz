// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: hard
///
/// The same as `Combo` but inherits `NoiseBase`, so the fuzzer must reach
/// the same highest value while 20 external functions revert or mutate
/// state that does not affect the value.
///
/// The reward of 1000 is only paid when `open`, `grab`, and `claim` run in
/// that exact order.
contract ComboWithNoise is NoiseBase {
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

    function value() external view returns (uint256) {
        return reward;
    }
}
