// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract RollTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_NUMBER = 42;

    uint256 public storedBlockNumber;

    function setup() external {
        vm.roll(EXPECTED_NUMBER);
        storedBlockNumber = block.number;
    }

    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }

    function getStoredBlockNumber() external view returns (uint256) {
        return storedBlockNumber;
    }

    /// Call vm.roll with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callRollSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.roll(EXPECTED_NUMBER);
        first = block.number;
        vm.roll(EXPECTED_NUMBER);
        second = block.number;
    }

    /// Call vm.roll with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callRollSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.roll(1);
        first = block.number;
        vm.roll(EXPECTED_NUMBER);
        second = block.number;
        vm.roll(5);
        third = block.number;
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callRollAndWarp()
        external
        returns (uint256 number, uint256 timestamp)
    {
        vm.roll(EXPECTED_NUMBER);
        vm.warp(1234567890);
        number = block.number;
        timestamp = block.timestamp;
    }

    /// Edge case: roll to a very large block number.
    function callRollLargeNumber() external returns (uint256 number) {
        vm.roll(type(uint256).max);
        number = block.number;
    }

    /// Fuzzing action: re-set the block number and store it.
    function actionRoll() external {
        vm.roll(EXPECTED_NUMBER);
        storedBlockNumber = block.number;
    }

    function invariant_roll() external view {
        assert(storedBlockNumber == EXPECTED_NUMBER);
    }
}
