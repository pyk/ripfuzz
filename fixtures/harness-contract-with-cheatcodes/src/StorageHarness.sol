// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz store/load cheatcodes.
///
/// Setup writes a canonical value to this contract's storage via vm.store
/// and reads it back via vm.load. Actions mutate storage; invariants verify
/// the canonical value is intact.
contract StorageHarness {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    bytes32 constant SLOT = bytes32(uint256(0));
    bytes32 constant EXPECTED = bytes32(uint256(42));

    bytes32 public storedValue;

    function setup() external {
        vm.store(address(this), SLOT, EXPECTED);
        storedValue = vm.load(address(this), SLOT);
    }

    /// Read the slot via vm.load to prove load works in a transaction.
    function getLoadedValue() external view returns (bytes32) {
        return vm.load(address(this), SLOT);
    }

    /// Read an empty slot to prove uninitialized storage returns zero.
    function getEmptySlotValue() external view returns (bytes32) {
        return vm.load(address(this), bytes32(uint256(1)));
    }

    /// Re-store the expected value and read it back.
    function actionRestore() external {
        vm.store(address(this), SLOT, EXPECTED);
        storedValue = vm.load(address(this), SLOT);
    }

    /// Mutate the value to something else, then read it back.
    function actionMutate() external {
        vm.store(address(this), SLOT, bytes32(uint256(99)));
        storedValue = vm.load(address(this), SLOT);
    }

    /// Store and load multiple slots in one tx to prove sequence safety.
    function actionSequence() external {
        bytes32 slot1 = bytes32(uint256(1));
        bytes32 slot2 = bytes32(uint256(2));
        vm.store(address(this), slot1, bytes32(uint256(100)));
        vm.store(address(this), slot2, bytes32(uint256(200)));
        bytes32 a = vm.load(address(this), slot1);
        bytes32 b = vm.load(address(this), slot2);
        assert(a == bytes32(uint256(100)));
        assert(b == bytes32(uint256(200)));
    }

    /// vm.store to a precompile must revert.
    function actionStorePrecompile() external {
        vm.store(address(1), SLOT, EXPECTED);
    }

    /// vm.load from a precompile must revert.
    function actionLoadPrecompile() external view {
        vm.load(address(1), SLOT);
    }

    /// Invariant: storedValue must match the expected value.
    function invariant_valueMatch() external view {
        assert(storedValue == EXPECTED);
    }
}
