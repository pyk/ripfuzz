// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: hard
///
/// The highest value is 5035, reached by calling `reduce`, `swap`, then
/// `increase` in that exact order.
///
/// The trajectory matches a value-delta ladder: `reduce` dumps the wallet
/// from 5008 to 923, `swap` is a flat rung that leaves the wallet unchanged,
/// and `increase` recovers to 5035. Ranking prefixes by current value loses
/// the dip, so the search has to keep negative and recovering deltas.
contract Ladder {
    uint256 internal step;
    uint256 internal wallet;

    constructor() {
        wallet = 5008;
    }

    function reduce() external {
        require(step == 0);
        step = 1;
        wallet = 923;
    }

    function swap() external {
        require(step == 1);
        step = 2;
    }

    function increase() external {
        require(step == 2);
        step = 3;
        wallet = 5035;
    }

    function value() external view returns (uint256) {
        return wallet;
    }
}
