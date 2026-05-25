// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract WarpTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_TIMESTAMP = 1_234_567_890;

    uint256 public storedBlockTimestamp;

    function setup() external {
        vm.warp(EXPECTED_TIMESTAMP);
        storedBlockTimestamp = block.timestamp;
    }

    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }

    function getStoredBlockTimestamp() external view returns (uint256) {
        return storedBlockTimestamp;
    }

    /// Call vm.warp with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callWarpSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.warp(EXPECTED_TIMESTAMP);
        first = block.timestamp;
        vm.warp(EXPECTED_TIMESTAMP);
        second = block.timestamp;
    }

    /// Call vm.warp with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callWarpSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.warp(1);
        first = block.timestamp;
        vm.warp(EXPECTED_TIMESTAMP);
        second = block.timestamp;
        vm.warp(5);
        third = block.timestamp;
    }

    /// Interaction with roll - both cheatcodes in same tx.
    function callWarpAndRoll()
        external
        returns (uint256 timestamp, uint256 number)
    {
        vm.warp(EXPECTED_TIMESTAMP);
        vm.roll(42);
        timestamp = block.timestamp;
        number = block.number;
    }

    /// Edge case: warp to a very large timestamp.
    function callWarpLargeNumber() external returns (uint256 timestamp) {
        vm.warp(type(uint256).max);
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-set the block timestamp and store it.
    function actionWarp() external {
        vm.warp(EXPECTED_TIMESTAMP);
        storedBlockTimestamp = block.timestamp;
    }

    function invariant_warp() external view {
        assert(block.timestamp == EXPECTED_TIMESTAMP);
    }
}
