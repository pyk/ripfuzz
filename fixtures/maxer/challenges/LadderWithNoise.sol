// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: hard
///
/// The same as `Ladder` but inherits `NoiseBase`, so the fuzzer must reach
/// the same highest value while 20 external functions revert or mutate
/// state that does not affect the value.
///
/// The highest value is 5035, reached by calling `reduce`, `swap`, then
/// `increase` in that exact order. `reduce` dumps the wallet from 5008 to
/// 923, `swap` is a flat rung, and `increase` recovers to 5035.
contract LadderWithNoise is NoiseBase {
    uint256 internal step;
    uint256 internal wallet;
    uint256 internal snap;

    constructor() {
        wallet = 5008;
    }

    function reduce() external {
        require(step == 0);
        step = 1;
        wallet = 923;
        snap = unused;
    }

    function swap() external {
        require(step == 1);
        require(unused == snap);
        step = 2;
    }

    function increase() external {
        require(step == 2);
        require(unused == snap);
        step = 3;
        wallet = 5035;
    }

    function value() external view returns (uint256) {
        return wallet;
    }
}
