// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz store/load cheatcodes.
///
/// Setup writes a canonical value to this contract's storage via rvm.store
/// and reads it back via rvm.load. Actions mutate storage; invariants verify
/// the canonical value is intact.
contract StorageHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    bytes32 constant SLOT = bytes32(uint256(0));
    bytes32 constant EXPECTED = bytes32(uint256(42));

    bytes32 public storedValue;

    function setup() external {
        rvm.store(address(this), SLOT, EXPECTED);
        storedValue = rvm.load(address(this), SLOT);
    }

    /// Read the slot via rvm.load to prove load works in a transaction.
    function getLoadedValue() external view returns (bytes32) {
        return rvm.load(address(this), SLOT);
    }

    /// Read an empty slot to prove uninitialized storage returns zero.
    function getEmptySlotValue() external view returns (bytes32) {
        return rvm.load(address(this), bytes32(uint256(1)));
    }

    /// Re-store the expected value and read it back.
    function actionRestore() external {
        rvm.store(address(this), SLOT, EXPECTED);
        storedValue = rvm.load(address(this), SLOT);
    }

    /// Mutate the value to something else, then read it back.
    function actionMutate() external {
        rvm.store(address(this), SLOT, bytes32(uint256(99)));
        storedValue = rvm.load(address(this), SLOT);
    }

    /// Store and load multiple slots in one tx to prove sequence safety.
    function actionSequence() external {
        bytes32 slot1 = bytes32(uint256(1));
        bytes32 slot2 = bytes32(uint256(2));
        rvm.store(address(this), slot1, bytes32(uint256(100)));
        rvm.store(address(this), slot2, bytes32(uint256(200)));
        bytes32 a = rvm.load(address(this), slot1);
        bytes32 b = rvm.load(address(this), slot2);
        assert(a == bytes32(uint256(100)));
        assert(b == bytes32(uint256(200)));
    }

    /// rvm.store to a precompile must revert.
    function actionStorePrecompile() external {
        rvm.store(address(1), SLOT, EXPECTED);
    }

    /// rvm.load from a precompile must revert.
    function actionLoadPrecompile() external view {
        rvm.load(address(1), SLOT);
    }

    /// Invariant: storedValue must match the expected value.
    function invariant_valueMatch() external view {
        assert(storedValue == EXPECTED);
    }
}
