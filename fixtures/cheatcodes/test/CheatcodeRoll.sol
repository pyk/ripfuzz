// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeRoll {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));
    uint256 public recordedBlockNumber;
    uint256 public recordedTimestamp;

    // --- setup interaction ---

    function setup() external {
        vm.roll(12345);
    }

    function call_record_block_number() external {
        recordedBlockNumber = block.number;
    }

    function call_record_timestamp() external {
        recordedTimestamp = block.timestamp;
    }

    function setup_roll_persists() external view returns (bool) {
        return recordedBlockNumber == 12345;
    }

    function setup_only() external view returns (bool) {
        return block.number == 12345;
    }

    // --- Same-sequence persistence ---

    function call_roll(uint256 num) external {
        vm.roll(num);
        recordedBlockNumber = block.number;
    }

    function roll_persists_across_calls() external view returns (bool) {
        // call_roll(100) -> roll to 100, then advance_block adds 1 for next call
        return recordedBlockNumber == 101;
    }

    // --- Revert safety ---

    function call_roll_and_revert(uint256 num) external {
        vm.roll(num);
        revert("intentional");
    }

    function revert_undoes_roll() external view returns (bool) {
        return block.number != 9999;
    }

    // --- Delay interaction ---

    function call_roll_100() external {
        vm.roll(100);
    }

    function roll_with_delay() external view returns (bool) {
        // call_roll_100() at idx=0, then call_record_block_number() at idx=1 with delay=5
        // advance_block adds 5, so expected 105
        return block.number == 105;
    }

    // --- Roll overwrite ---

    function call_roll_200() external {
        vm.roll(200);
    }

    function roll_overwrite() external view returns (bool) {
        // call_roll_100() -> 100, call_roll_200() -> 200, then advance_block adds 1
        return block.number == 201;
    }

    // --- Edge: roll to zero ---

    function call_roll_zero() external {
        vm.roll(0);
    }

    function roll_zero() external view returns (bool) {
        // roll to 0 at idx=0, then advance_block adds 1 for idx=1
        return block.number == 1;
    }

    // --- Edge: roll to max uint64 ---

    function call_roll_max_uint64() external {
        vm.roll(type(uint64).max);
    }

    function roll_max_uint64() external view returns (bool) {
        return block.number == type(uint64).max;
    }

    // --- Property sees final roll ---

    function final_block_number() external view returns (bool) {
        // If the only call was call_roll_100(), the property should see 100
        return block.number == 100;
    }

    // --- Cross-cheatcode interaction: roll + warp ---

    function call_roll_and_warp() external {
        vm.roll(5000);
        vm.warp(1000);
    }

    function roll_and_warp() external view returns (bool) {
        return block.number == 5000 && block.timestamp == 1000;
    }
}
