// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract FeeTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_BASEFEE = 42;

    uint256 public storedBasefee;

    function setup() external {
        vm.fee(EXPECTED_BASEFEE);
        storedBasefee = block.basefee;
    }

    function getBasefee() external view returns (uint256) {
        return block.basefee;
    }

    function getStoredBasefee() external view returns (uint256) {
        return storedBasefee;
    }

    /// Call vm.fee with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callFeeSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.fee(EXPECTED_BASEFEE);
        first = block.basefee;
        vm.fee(EXPECTED_BASEFEE);
        second = block.basefee;
    }

    /// Call vm.fee with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callFeeSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.fee(1);
        first = block.basefee;
        vm.fee(EXPECTED_BASEFEE);
        second = block.basefee;
        vm.fee(5);
        third = block.basefee;
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callFeeAndWarp()
        external
        returns (uint256 basefee, uint256 timestamp)
    {
        vm.fee(EXPECTED_BASEFEE);
        vm.warp(1234567890);
        basefee = block.basefee;
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-set the basefee and store it.
    function actionFee() external {
        vm.fee(EXPECTED_BASEFEE);
        storedBasefee = block.basefee;
    }

    function invariant_fee() external view {
        assert(storedBasefee == EXPECTED_BASEFEE);
    }
}
