// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title FeeTarget
/// @notice Real-world fuzzing target that controls `block.basefee` via the
///         `vm.fee` cheatcode. Setup establishes a canonical basefee and
///         actions mutate or restore it. Invariants verify deterministic control.
contract FeeTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_BASEFEE = 42;

    function setup() external {
        vm.fee(EXPECTED_BASEFEE);
    }

    /// Invariant: the live basefee must always match the expected value.
    function invariant_fee() external view {
        assert(block.basefee == EXPECTED_BASEFEE);
    }

    /// Action: re-set the basefee to the expected value.
    function actionRestoreFee() external {
        vm.fee(EXPECTED_BASEFEE);
    }

    /// Action: temporarily set a different basefee.
    function actionMutateFee() external {
        vm.fee(1337);
    }

    /// Action: interleave basefee changes inside one tx, ending on expected.
    function actionFeeSequence() external {
        vm.fee(1);
        vm.fee(EXPECTED_BASEFEE);
        vm.fee(5);
        vm.fee(EXPECTED_BASEFEE);
    }
}
