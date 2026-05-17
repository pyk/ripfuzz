// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeDifficulty {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public recordedDifficulty;

    // --- setUp interaction ---

    function setUp() external {
        vm.difficulty(9999);
    }

    function action_record() external {
        recordedDifficulty = block.difficulty;
    }

    function property_setup_difficulty_unchanged() external view returns (bool) {
        // setUp() called vm.difficulty(9999) but it must be a no-op.
        // On post-Paris, block.difficulty reads prevrandao, which defaults to 0.
        return recordedDifficulty == 0;
    }

    function property_setup_only() external view returns (bool) {
        return block.difficulty == 0;
    }

    // --- Same-sequence no-op ---

    function action_difficulty(uint256 x) external {
        vm.difficulty(x);
        recordedDifficulty = block.difficulty;
    }

    function property_difficulty_no_op() external view returns (bool) {
        // action_difficulty(12345) did nothing, so block.difficulty is still 0.
        return recordedDifficulty == 0;
    }

    // --- Revert safety ---

    function action_difficulty_and_revert(uint256 x) external {
        vm.difficulty(x);
        revert("intentional");
    }

    function property_revert_is_still_no_op() external view returns (bool) {
        // Even if the call reverted, there was nothing to undo.
        return block.difficulty == 0;
    }

    // --- Interaction with prevrandao ---

    function action_prevrandao_then_difficulty() external {
        vm.prevrandao(bytes32(uint256(42)));
        vm.difficulty(9999);
        recordedDifficulty = block.difficulty;
    }

    function property_prevrandao_unaffected() external view returns (bool) {
        // prevrandao was set to 42; difficulty(9999) must not overwrite it.
        // On post-Paris, block.difficulty == prevrandao.
        return recordedDifficulty == 42;
    }

    // --- Overwrite (multiple no-ops) ---

    function property_difficulty_still_unchanged() external view returns (bool) {
        // action_difficulty(1) -> action_difficulty(2) -> action_record
        // Both calls are no-ops; recordedDifficulty remains 0.
        return recordedDifficulty == 0;
    }

    // --- Edge: difficulty to zero ---

    function action_difficulty_zero() external {
        vm.difficulty(0);
    }

    function property_difficulty_zero_no_op() external view returns (bool) {
        return block.difficulty == 0;
    }

    // --- Edge: difficulty to max uint64 ---

    function action_difficulty_max() external {
        vm.difficulty(type(uint64).max);
    }

    function property_difficulty_max_no_op() external view returns (bool) {
        return block.difficulty == 0;
    }

    // --- Property sees default difficulty ---

    function action_noop() external {
        // Does nothing; property checks the default state.
    }

    function property_final_difficulty() external view returns (bool) {
        return block.difficulty == 0;
    }
}
