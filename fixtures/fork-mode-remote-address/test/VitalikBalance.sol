// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract VitalikBalance {
    /// vitalik.eth balance at block 25_259_523.
    uint256 internal constant EXPECTED = 5_688_184_733_246_745_254;

    bool public asserted;

    constructor() {
        uint256 bal = address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045)
            .balance;
        assert(bal == EXPECTED);
        asserted = true;
    }
}
