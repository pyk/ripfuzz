// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";

/// @notice Regression fixture for verifying that fork mode correctly fetches
/// and caches remote account data. The contract reads vitalik.eth balance in
/// four different execution paths -- constructor, setup, handler function, and
/// invariant -- to confirm that the cached value is consistent and that no
/// redundant RPC calls occur after the initial fetch.
contract RemoteAccountBalance {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// vitalik.eth balance at block 25_259_523.
    uint256 internal constant EXPECTED = 5_688_240_446_715_981_478;

    bool public asserted;

    constructor() {
        rvm.fork("mock://test", 25_259_523);
        uint256 bal = address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045).balance;
        assert(bal == EXPECTED);
        asserted = true;
    }

    function setup() external {
        uint256 bal = address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045).balance;
        assert(bal == EXPECTED);
    }

    function checkBalance() external {
        uint256 bal = address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045).balance;
        assert(bal == EXPECTED);
    }

    function invariant_checkBalance() external view {
        uint256 bal = address(0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045).balance;
        assert(bal == EXPECTED);
    }
}
