// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./Counter.sol";
import "./AltCounter.sol";

/// @title EtchTarget
/// @notice Real-world fuzzing target that deploys mock bytecode via the
///     `vm.etch` cheatcode. Setup establishes a canonical contract and
///     actions mutate or restore it. Invariants verify deterministic control.
contract EtchTarget {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address constant ETCH_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    function setup() external {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }

    /// Invariant: the etched contract must always return the expected value.
    function invariant_etch() external view {
        assert(Counter(ETCH_ADDR).getValue() == 42);
    }

    /// Action: re-etch the canonical contract at the target address.
    function actionRestoreEtch() external {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }

    /// Action: temporarily etch a different contract at the target address.
    function actionMutateEtch() external {
        vm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
    }

    /// Action: interleave etch changes inside one tx, ending on expected.
    function actionEtchSequence() external {
        vm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        vm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
    }
}
