// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeStore {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));

    bytes32 public constant SLOT_A = bytes32(uint256(1));
    bytes32 public constant SLOT_B = bytes32(uint256(2));
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);

    bytes32 public recordedValue;

    // --- setUp interaction ---

    function setUp() external {
        vm.store(address(this), SLOT_A, bytes32(uint256(0xCAFE)));
        vm.store(TARGET, SLOT_A, bytes32(uint256(0xBABE)));
    }

    function call_record_slot_a() external {
        recordedValue = vm.load(address(this), SLOT_A);
    }

    function setup_store_persists() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE))
            && vm.load(TARGET, SLOT_A) == bytes32(uint256(0xBABE));
    }

    // --- Same-sequence persistence ---

    function call_store_then_load(bytes32 value) external {
        vm.store(TARGET, SLOT_A, value);
        recordedValue = vm.load(TARGET, SLOT_A);
    }

    function store_persists_across_calls() external view returns (bool) {
        return recordedValue == bytes32(uint256(0xFACADE));
    }

    // --- Revert safety ---

    function call_store_and_revert(bytes32 value) external {
        vm.store(TARGET, SLOT_B, value);
        revert("intentional");
    }

    function revert_undoes_store() external view returns (bool) {
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

    // --- Zero write (clear slot) ---

    function call_store_zero() external {
        vm.store(TARGET, SLOT_A, bytes32(0));
    }

    function store_zero_clears() external view returns (bool) {
        return vm.load(TARGET, SLOT_A) == bytes32(0);
    }

    // --- Empty / non-existent address ---

    function call_store_empty() external {
        vm.store(EMPTY_ADDR, SLOT_A, bytes32(uint256(0xFACE)));
        recordedValue = vm.load(EMPTY_ADDR, SLOT_A);
    }

    function store_empty_address() external view returns (bool) {
        return recordedValue == bytes32(uint256(0xFACE));
    }

    // --- Multi-call sequence final state ---

    function call_store_step1() external {
        vm.store(address(this), SLOT_B, bytes32(uint256(0xAAAA)));
    }

    function call_store_step2() external {
        vm.store(address(this), SLOT_B, bytes32(uint256(0xBBBB)));
    }

    function multi_call_final_state() external view returns (bool) {
        return vm.load(address(this), SLOT_B) == bytes32(uint256(0xBBBB));
    }

    // --- Precompile rejection ---

    function call_store_precompile() external {
        vm.store(address(0x01), SLOT_A, bytes32(uint256(0xBAD)));
    }

    function precompile_store_reverts() external view returns (bool) {
        // This property should never be reached if the precompile store reverts.
        // We include a trivial true so the fixture compiles; the test asserts
        // that the call reverts instead of checking this property.
        return true;
    }

    // --- Cross-cheatcode interference ---

    function call_store_and_warp() external {
        vm.store(address(this), SLOT_A, bytes32(uint256(0x9999)));
        vm.warp(12345);
    }

    function store_and_warp() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0x9999))
            && block.timestamp == 12345;
    }

    // --- Corpus isolation helper ---

    function setup_only_store() external view returns (bool) {
        return vm.load(address(this), SLOT_A) == bytes32(uint256(0xCAFE));
    }
}
