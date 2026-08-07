// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @title FeeHarness
/// @notice Real-world fuzz handler that controls `block.basefee` via the
///         `rvm.fee` cheatcode. Setup establishes a canonical basefee and
///         actions mutate or restore it. Invariants verify deterministic control.
contract FeeHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    uint256 constant EXPECTED_BASEFEE = 42;

    function setup() external {
        rvm.fee(EXPECTED_BASEFEE);
    }

    /// Invariant: the live basefee must always match the expected value.
    function invariant_fee() external view {
        assert(block.basefee == EXPECTED_BASEFEE);
    }

    /// Action: re-set the basefee to the expected value.
    function actionRestoreFee() external {
        rvm.fee(EXPECTED_BASEFEE);
    }

    /// Action: temporarily set a different basefee.
    function actionMutateFee() external {
        rvm.fee(1337);
    }

    /// Action: interleave basefee changes inside one tx, ending on expected.
    function actionFeeSequence() external {
        rvm.fee(1);
        rvm.fee(EXPECTED_BASEFEE);
        rvm.fee(5);
        rvm.fee(EXPECTED_BASEFEE);
    }
}
