// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeWarp {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));
    uint256 public recordedTimestamp;
    uint256 public recordedBlockNumber;

    // --- setUp interaction ---

    function setUp() external {
        vm.warp(1234567890);
    }

    function call_record_timestamp() external {
        recordedTimestamp = block.timestamp;
    }

    function call_record_block_number() external {
        recordedBlockNumber = block.number;
    }

    function setup_warp_persists() external view returns (bool) {
        return recordedTimestamp == 1234567890;
    }

    function setup_only() external view returns (bool) {
        return block.timestamp == 1234567890;
    }

    // --- Same-sequence persistence ---

    function call_warp(uint256 ts) external {
        vm.warp(ts);
        recordedTimestamp = block.timestamp;
    }

    function warp_persists_across_calls() external view returns (bool) {
        // call_warp(100) -> warp to 100, then advance_block adds 1 for next call
        return recordedTimestamp == 101;
    }

    // --- Revert safety ---

    function call_warp_and_revert(uint256 ts) external {
        vm.warp(ts);
        revert("intentional");
    }

    function revert_undoes_warp() external view returns (bool) {
        return block.timestamp != 9999;
    }

    // --- Delay interaction ---

    function call_warp_100() external {
        vm.warp(100);
    }

    function warp_with_delay() external view returns (bool) {
        // call_warp_100() at idx=0, then call_record_timestamp() at idx=1 with delay=5
        // advance_block adds 5, so expected 105
        return block.timestamp == 105;
    }

    // --- Warp overwrite ---

    function call_warp_200() external {
        vm.warp(200);
    }

    function warp_overwrite() external view returns (bool) {
        // call_warp_100() -> 100, call_warp_200() -> 200, then advance_block adds 1
        return block.timestamp == 201;
    }

    // --- Edge: warp to zero ---

    function call_warp_zero() external {
        vm.warp(0);
    }

    function warp_zero() external view returns (bool) {
        // warp to 0 at idx=0, then advance_block adds 1 for idx=1
        return block.timestamp == 1;
    }

    // --- Edge: warp to max uint64 ---

    function call_warp_max_uint64() external {
        vm.warp(type(uint64).max);
    }

    function warp_max_uint64() external view returns (bool) {
        return block.timestamp == type(uint64).max;
    }

    // --- Property sees final warp ---

    function final_timestamp() external view returns (bool) {
        // If the only call was call_warp_100(), the property should see 100
        return block.timestamp == 100;
    }

    // --- Cross-cheatcode interaction: warp + roll ---

    function call_warp_and_roll() external {
        vm.warp(1000);
        vm.roll(5000);
    }

    function warp_and_roll() external view returns (bool) {
        return block.timestamp == 1000 && block.number == 5000;
    }
}
