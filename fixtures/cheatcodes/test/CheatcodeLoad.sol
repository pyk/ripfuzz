// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeLoad {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    bytes32 public constant SLOT_A = bytes32(uint256(1));
    bytes32 public constant SLOT_B = bytes32(uint256(2));
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);

    bytes32 public recordedValue;

    // --- setup interaction ---

    function setup() external {
        vm.store(address(this), SLOT_A, bytes32(uint256(0xCAFE)));
        vm.store(TARGET, SLOT_A, bytes32(uint256(0xBABE)));
    }

    function call_record_slot_a() external {
        recordedValue = vm.load(address(this), SLOT_A);
    }

    function call_record_target_slot_a() external {
        recordedValue = vm.load(TARGET, SLOT_A);
    }

    function setup_load_persists() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE))
            && vm.load(TARGET, SLOT_A) == bytes32(uint256(0xBABE));
    }

    function setup_only() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE));
    }

    // --- Same-sequence persistence ---

    function call_store_then_load(bytes32 value) external {
        vm.store(TARGET, SLOT_A, value);
        recordedValue = vm.load(TARGET, SLOT_A);
    }

    function store_load_persists_across_calls() external view returns (bool) {
        return recordedValue == bytes32(uint256(0xFACADE));
    }

    // --- Revert safety ---

    function call_store_and_revert(bytes32 value) external {
        vm.store(TARGET, SLOT_B, value);
        revert("intentional");
    }

    function revert_undoes_store() external view returns (bool) {
        // setup stored 0xBABE in SLOT_A of TARGET; SLOT_B was never touched.
        // If vm.store is rolled back on revert, SLOT_B must be zero.
        return vm.load(TARGET, SLOT_B) == bytes32(0);
    }

    // --- Overwrite ---

    function call_store_overwrite() external {
        vm.store(TARGET, SLOT_A, bytes32(uint256(0x1111)));
        vm.store(TARGET, SLOT_A, bytes32(uint256(0x2222)));
    }

    function store_overwrite() external view returns (bool) {
        return vm.load(TARGET, SLOT_A) == bytes32(uint256(0x2222));
    }

    // --- Empty / non-existent address ---

    function call_load_empty() external {
        recordedValue = vm.load(EMPTY_ADDR, SLOT_A);
    }

    function load_empty_returns_zero() external view returns (bool) {
        return recordedValue == bytes32(0);
    }

    // --- Property sees final state ---

    function final_load() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE));
    }

    // --- Cross-cheatcode interaction: load + deal + warp ---

    function call_load_and_warp() external {
        vm.store(address(this), SLOT_A, bytes32(uint256(0x9999)));
        vm.warp(12345);
    }

    function load_and_warp() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0x9999))
            && block.timestamp == 12345;
    }

    // --- Precompile rejection ---

    function call_load_precompile() external view {
        vm.load(address(0x01), SLOT_A);
    }
}
