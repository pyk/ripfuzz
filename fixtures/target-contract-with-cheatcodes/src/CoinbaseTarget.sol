// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract CoinbaseTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant EXPECTED_COINBASE = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    address public storedCoinbase;

    function setup() external {
        vm.coinbase(EXPECTED_COINBASE);
        storedCoinbase = block.coinbase;
    }

    function getCoinbase() external view returns (address) {
        return block.coinbase;
    }

    function getStoredCoinbase() external view returns (address) {
        return storedCoinbase;
    }

    /// Call vm.coinbase with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callCoinbaseSameValueTwice() external returns (address first, address second) {
        vm.coinbase(EXPECTED_COINBASE);
        first = block.coinbase;
        vm.coinbase(EXPECTED_COINBASE);
        second = block.coinbase;
    }

    /// Call vm.coinbase with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callCoinbaseSequence()
        external
        returns (address first, address second, address third)
    {
        vm.coinbase(address(0x1111111111111111111111111111111111111111));
        first = block.coinbase;
        vm.coinbase(EXPECTED_COINBASE);
        second = block.coinbase;
        vm.coinbase(address(0x2222222222222222222222222222222222222222));
        third = block.coinbase;
    }

    /// Fuzzing action: re-set the coinbase and store it.
    function actionCoinbase() external {
        vm.coinbase(EXPECTED_COINBASE);
        storedCoinbase = block.coinbase;
    }

    function invariant_coinbase() external view {
        assert(storedCoinbase == EXPECTED_COINBASE);
    }
}
