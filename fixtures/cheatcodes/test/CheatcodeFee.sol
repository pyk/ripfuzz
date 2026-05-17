// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeFee {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public recordedBaseFee;
    uint256 public recordedBlockNumber;
    uint256 public recordedTimestamp;

    // --- setUp interaction ---

    function setUp() external {
        vm.fee(12345);
    }

    function action_record_basefee() external {
        recordedBaseFee = block.basefee;
    }

    function action_record_block_number() external {
        recordedBlockNumber = block.number;
    }

    function action_record_timestamp() external {
        recordedTimestamp = block.timestamp;
    }

    function property_setup_fee_persists() external view returns (bool) {
        return recordedBaseFee == 12345;
    }

    function property_setup_only() external view returns (bool) {
        return block.basefee == 12345;
    }

    // --- Same-sequence persistence ---

    function action_fee(uint256 num) external {
        vm.fee(num);
        recordedBaseFee = block.basefee;
    }

    function property_fee_persists_across_calls() external view returns (bool) {
        // action_fee(100) -> fee = 100, next call sees 100 (no auto-advance)
        return recordedBaseFee == 100;
    }

    // --- Revert safety ---

    function action_fee_and_revert(uint256 num) external {
        vm.fee(num);
        revert("intentional");
    }

    function property_revert_undoes_fee() external view returns (bool) {
        return block.basefee != 9999;
    }

    // --- Fee overwrite ---

    function action_fee_100() external {
        vm.fee(100);
    }

    function action_fee_200() external {
        vm.fee(200);
    }

    function property_fee_overwrite() external view returns (bool) {
        // action_fee_100 -> 100, action_fee_200 -> 200
        return block.basefee == 200;
    }

    // --- Edge: fee to zero ---

    function action_fee_zero() external {
        vm.fee(0);
    }

    function property_fee_zero() external view returns (bool) {
        return block.basefee == 0;
    }

    // --- Edge: fee to max uint64 ---

    function action_fee_max_uint64() external {
        vm.fee(type(uint64).max);
    }

    function property_fee_max_uint64() external view returns (bool) {
        return block.basefee == type(uint64).max;
    }

    // --- Property sees final fee ---

    function property_final_basefee() external view returns (bool) {
        // If the only call was action_fee_100(), the property should see 100
        return block.basefee == 100;
    }

    // --- Cross-cheatcode interaction: fee + roll + warp ---

    function action_fee_and_roll_warp() external {
        vm.fee(5000);
        vm.roll(7000);
        vm.warp(9000);
    }

    function property_fee_and_roll_warp() external view returns (bool) {
        return block.basefee == 5000
            && block.number == 7000
            && block.timestamp == 9000;
    }
}
