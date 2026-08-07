// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeLoad {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    bytes32 public constant SLOT_A = bytes32(uint256(1));
    bytes32 public constant SLOT_B = bytes32(uint256(2));
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);

    bytes32 public recordedValue;

    // --- setup interaction ---

    function setup() external {
        rvm.store(address(this), SLOT_A, bytes32(uint256(0xCAFE)));
        rvm.store(TARGET, SLOT_A, bytes32(uint256(0xBABE)));
    }

    function call_record_slot_a() external {
        recordedValue = rvm.load(address(this), SLOT_A);
    }

    function call_record_target_slot_a() external {
        recordedValue = rvm.load(TARGET, SLOT_A);
    }

    function setup_load_persists() external view returns (bool) {
        return rvm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE))
            && rvm.load(TARGET, SLOT_A) == bytes32(uint256(0xBABE));
    }

    function setup_only() external view returns (bool) {
        return rvm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE));
    }

    // --- Same-sequence persistence ---

    function call_store_then_load(bytes32 value) external {
        rvm.store(TARGET, SLOT_A, value);
        recordedValue = rvm.load(TARGET, SLOT_A);
    }

    function store_load_persists_across_calls() external view returns (bool) {
        return recordedValue == bytes32(uint256(0xFACADE));
    }

    // --- Revert safety ---

    function call_store_and_revert(bytes32 value) external {
        rvm.store(TARGET, SLOT_B, value);
        revert("intentional");
    }

    function revert_undoes_store() external view returns (bool) {
        // setup stored 0xBABE in SLOT_A of TARGET; SLOT_B was never touched.
        // If rvm.store is rolled back on revert, SLOT_B must be zero.
        return rvm.load(TARGET, SLOT_B) == bytes32(0);
    }

    // --- Overwrite ---

    function call_store_overwrite() external {
        rvm.store(TARGET, SLOT_A, bytes32(uint256(0x1111)));
        rvm.store(TARGET, SLOT_A, bytes32(uint256(0x2222)));
    }

    function store_overwrite() external view returns (bool) {
        return rvm.load(TARGET, SLOT_A) == bytes32(uint256(0x2222));
    }

    // --- Empty / non-existent address ---

    function call_load_empty() external {
        recordedValue = rvm.load(EMPTY_ADDR, SLOT_A);
    }

    function load_empty_returns_zero() external view returns (bool) {
        return recordedValue == bytes32(0);
    }

    // --- Property sees final state ---

    function final_load() external view returns (bool) {
        return rvm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE));
    }

    // --- Cross-cheatcode interaction: load + deal + warp ---

    function call_load_and_warp() external {
        rvm.store(address(this), SLOT_A, bytes32(uint256(0x9999)));
        rvm.warp(12345);
    }

    function load_and_warp() external view returns (bool) {
        return rvm.load(address(this), SLOT_A) == bytes32(uint256(0x9999))
            && block.timestamp == 12345;
    }

    // --- Precompile rejection ---

    function call_load_precompile() external view {
        rvm.load(address(0x01), SLOT_A);
    }
}
