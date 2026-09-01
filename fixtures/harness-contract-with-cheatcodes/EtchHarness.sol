// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";
import "./Counter.sol";
import "./AltCounter.sol";

/// @title EtchHarness
/// @notice Real-world fuzz handler that deploys mock bytecode via the
///     `rvm.etch` cheatcode. Setup establishes a canonical contract and
///     actions mutate or restore it. Invariants verify deterministic control.
contract EtchHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address constant ETCH_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    function setup() external {
        rvm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }

    /// Invariant: the etched contract must always return the expected value.
    function invariant_etch() external view {
        assert(Counter(ETCH_ADDR).getValue() == 42);
    }

    /// Action: re-etch the canonical contract at the target address.
    function actionRestoreEtch() external {
        rvm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }

    /// Action: temporarily etch a different contract at the target address.
    function actionMutateEtch() external {
        rvm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
    }

    /// Action: interleave etch changes inside one tx, ending on expected.
    function actionEtchSequence() external {
        rvm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
        rvm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        rvm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
        rvm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }
}
