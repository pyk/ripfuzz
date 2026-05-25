// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract StorageTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant TARGET_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    bytes32 constant SLOT = bytes32(uint256(0));
    bytes32 constant EXPECTED_VALUE = bytes32(uint256(42));

    bytes32 public storedValue;

    function setup() external {
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        storedValue = vm.load(TARGET_ADDR, SLOT);
    }

    function getStoredValue() external view returns (bytes32) {
        return storedValue;
    }

    function getLoadedValue() external view returns (bytes32) {
        return vm.load(TARGET_ADDR, SLOT);
    }

    function getEmptySlotValue() external view returns (bytes32) {
        return vm.load(TARGET_ADDR, bytes32(uint256(1)));
    }

    /// Call vm.store with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callStoreSameValueTwice()
        external
        returns (bytes32 first, bytes32 second)
    {
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        first = vm.load(TARGET_ADDR, SLOT);
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        second = vm.load(TARGET_ADDR, SLOT);
    }

    /// Call vm.store with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callStoreSequence()
        external
        returns (bytes32 first, bytes32 second, bytes32 third)
    {
        vm.store(TARGET_ADDR, SLOT, bytes32(uint256(1)));
        first = vm.load(TARGET_ADDR, SLOT);
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        second = vm.load(TARGET_ADDR, SLOT);
        vm.store(TARGET_ADDR, SLOT, bytes32(uint256(2)));
        third = vm.load(TARGET_ADDR, SLOT);
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callStoreAndWarp()
        external
        returns (bytes32 value, uint256 timestamp)
    {
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        vm.warp(1234567890);
        value = vm.load(TARGET_ADDR, SLOT);
        timestamp = block.timestamp;
    }

    /// vm.store to a precompile must revert.
    function callStoreToPrecompile() external {
        vm.store(address(1), SLOT, EXPECTED_VALUE);
    }

    /// vm.load from a precompile must revert.
    function callLoadFromPrecompile() external view {
        vm.load(address(1), SLOT);
    }

    /// Fuzzing action: re-store the expected value and load it back.
    function actionStore() external {
        vm.store(TARGET_ADDR, SLOT, EXPECTED_VALUE);
        storedValue = vm.load(TARGET_ADDR, SLOT);
    }

    function invariant_storage() external view {
        assert(storedValue == EXPECTED_VALUE);
    }
}
