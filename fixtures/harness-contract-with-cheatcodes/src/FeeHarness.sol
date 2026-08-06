// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title FeeHarness
/// @notice Real-world fuzz handler that controls `block.basefee` via the
///         `vm.fee` cheatcode. Setup establishes a canonical basefee and
///         actions mutate or restore it. Invariants verify deterministic control.
contract FeeHarness {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

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
