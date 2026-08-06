// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz roll cheatcode.
///
/// Setup establishes a canonical `block.number` via `vm.roll`.  Actions
/// mutate or restore the value; invariants verify the canonical state.
contract RollHandler {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant CANONICAL = 42;

    uint256 public storedBlockNumber;

    function setup() external {
        vm.roll(CANONICAL);
        storedBlockNumber = block.number;
    }

    /// Re-set the canonical block number and store it.
    function actionRestoreCanonical() external {
        vm.roll(CANONICAL);
        storedBlockNumber = block.number;
    }

    /// Set a non-canonical block number and store it.
    function actionMutateValue() external {
        vm.roll(999);
        storedBlockNumber = block.number;
    }

    /// Interleave multiple block numbers, ending on the canonical one.
    function actionSequence() external {
        vm.roll(1);
        vm.roll(2);
        vm.roll(CANONICAL);
        storedBlockNumber = block.number;
    }

    /// Read block.number without calling any cheatcode.  Proves the value
    /// set during setup persists across the exec via block_env.
    function actionReadBlockNumber() external {
        storedBlockNumber = block.number;
    }

    /// Directly return the current `block.number`.
    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }

    /// Read the stored block number.
    function getStoredBlockNumber() external view returns (uint256) {
        return storedBlockNumber;
    }

    /// Invariant: stored block number must match the canonical value.
    function invariant_blockNumberMatch() external view {
        assert(storedBlockNumber == CANONICAL);
    }
}
