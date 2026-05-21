// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeNonce {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));

    uint64 public recordedNonce;
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);
    address public constant EOA = address(0xCAFE);

    // --- setup interaction ---

    function setup() external {
        vm.setNonce(address(this), 7);
        vm.setNonce(TARGET, 5);
    }

    function call_record_nonce() external {
        recordedNonce = vm.getNonce(address(this));
    }

    function call_record_target_nonce() external {
        recordedNonce = vm.getNonce(TARGET);
    }

    function setup_nonce_persists() external view returns (bool) {
        return vm.getNonce(address(this)) == 7 && vm.getNonce(TARGET) == 5;
    }

    function setup_only() external view returns (bool) {
        return vm.getNonce(address(this)) == 7;
    }

    // --- Same-sequence persistence ---

    function call_set_nonce(uint64 nonce) external {
        vm.setNonce(TARGET, nonce);
        recordedNonce = vm.getNonce(TARGET);
    }

    function nonce_persists_across_calls() external view returns (bool) {
        return recordedNonce == 100;
    }

    // --- Revert safety ---

    function call_set_nonce_and_revert(uint64 nonce) external {
        vm.setNonce(TARGET, nonce);
        revert("intentional");
    }

    function revert_undoes_nonce() external view returns (bool) {
        return vm.getNonce(TARGET) == 5;
    }

    // --- Validation ---

    function call_set_nonce_invalid() external {
        vm.setNonce(TARGET, 1); // current is 5, so 1 < 5 must revert
    }

    function invalid_nonce_reverted() external view returns (bool) {
        return vm.getNonce(TARGET) == 5;
    }

    // --- Overwrite ---

    function call_set_nonce_100() external {
        vm.setNonce(TARGET, 100);
    }

    function call_set_nonce_200() external {
        vm.setNonce(TARGET, 200);
    }

    function nonce_overwrite() external view returns (bool) {
        return vm.getNonce(TARGET) == 200;
    }

    // --- Edge: zero ---

    function call_set_nonce_zero() external {
        vm.setNonce(EMPTY_ADDR, 0);
    }

    function nonce_zero() external view returns (bool) {
        return vm.getNonce(EMPTY_ADDR) == 0;
    }

    // --- Edge: max uint64 ---

    function call_set_nonce_max() external {
        vm.setNonce(TARGET, type(uint64).max);
    }

    function nonce_max() external view returns (bool) {
        return vm.getNonce(TARGET) == type(uint64).max;
    }

    // --- Edge: empty address ---

    function call_set_nonce_empty(uint64 nonce) external {
        vm.setNonce(EMPTY_ADDR, nonce);
    }

    function nonce_empty() external view returns (bool) {
        return vm.getNonce(EMPTY_ADDR) == 42;
    }

    // --- Edge: EOA ---

    function call_set_nonce_eoa(uint64 nonce) external {
        vm.setNonce(EOA, nonce);
    }

    function nonce_eoa() external view returns (bool) {
        return vm.getNonce(EOA) == 99;
    }

    // --- Property sees final nonce ---

    function final_nonce() external view returns (bool) {
        return vm.getNonce(TARGET) == 100;
    }

    // --- Cross-cheatcode interaction ---

    function call_set_nonce_and_warp_roll() external {
        vm.setNonce(TARGET, 333);
        vm.warp(12345);
        vm.roll(67890);
    }

    function nonce_and_warp_roll() external view returns (bool) {
        return vm.getNonce(TARGET) == 333
            && block.timestamp == 12345
            && block.number == 67890;
    }

    // --- Self-setNonce overwrites setup ---

    function call_self_set_nonce(uint64 nonce) external {
        vm.setNonce(address(this), nonce);
    }

    function self_set_nonce_overwrites_setup() external view returns (bool) {
        return vm.getNonce(address(this)) == 50;
    }
}
