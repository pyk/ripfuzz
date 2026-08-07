// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeNonce {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    uint64 public recordedNonce;
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);
    address public constant EOA = address(0xCAFE);

    // --- setup interaction ---

    function setup() external {
        rvm.setNonce(address(this), 7);
        rvm.setNonce(TARGET, 5);
    }

    function call_record_nonce() external {
        recordedNonce = rvm.getNonce(address(this));
    }

    function call_record_target_nonce() external {
        recordedNonce = rvm.getNonce(TARGET);
    }

    function setup_nonce_persists() external view returns (bool) {
        return rvm.getNonce(address(this)) == 7 && rvm.getNonce(TARGET) == 5;
    }

    function setup_only() external view returns (bool) {
        return rvm.getNonce(address(this)) == 7;
    }

    // --- Same-sequence persistence ---

    function call_set_nonce(uint64 nonce) external {
        rvm.setNonce(TARGET, nonce);
        recordedNonce = rvm.getNonce(TARGET);
    }

    function nonce_persists_across_calls() external view returns (bool) {
        return recordedNonce == 100;
    }

    // --- Revert safety ---

    function call_set_nonce_and_revert(uint64 nonce) external {
        rvm.setNonce(TARGET, nonce);
        revert("intentional");
    }

    function revert_undoes_nonce() external view returns (bool) {
        return rvm.getNonce(TARGET) == 5;
    }

    // --- Validation ---

    function call_set_nonce_invalid() external {
        rvm.setNonce(TARGET, 1); // current is 5, so 1 < 5 must revert
    }

    function invalid_nonce_reverted() external view returns (bool) {
        return rvm.getNonce(TARGET) == 5;
    }

    // --- Overwrite ---

    function call_set_nonce_100() external {
        rvm.setNonce(TARGET, 100);
    }

    function call_set_nonce_200() external {
        rvm.setNonce(TARGET, 200);
    }

    function nonce_overwrite() external view returns (bool) {
        return rvm.getNonce(TARGET) == 200;
    }

    // --- Edge: zero ---

    function call_set_nonce_zero() external {
        rvm.setNonce(EMPTY_ADDR, 0);
    }

    function nonce_zero() external view returns (bool) {
        return rvm.getNonce(EMPTY_ADDR) == 0;
    }

    // --- Edge: max uint64 ---

    function call_set_nonce_max() external {
        rvm.setNonce(TARGET, type(uint64).max);
    }

    function nonce_max() external view returns (bool) {
        return rvm.getNonce(TARGET) == type(uint64).max;
    }

    // --- Edge: empty address ---

    function call_set_nonce_empty(uint64 nonce) external {
        rvm.setNonce(EMPTY_ADDR, nonce);
    }

    function nonce_empty() external view returns (bool) {
        return rvm.getNonce(EMPTY_ADDR) == 42;
    }

    // --- Edge: EOA ---

    function call_set_nonce_eoa(uint64 nonce) external {
        rvm.setNonce(EOA, nonce);
    }

    function nonce_eoa() external view returns (bool) {
        return rvm.getNonce(EOA) == 99;
    }

    // --- Property sees final nonce ---

    function final_nonce() external view returns (bool) {
        return rvm.getNonce(TARGET) == 100;
    }

    // --- Cross-cheatcode interaction ---

    function call_set_nonce_and_warp_roll() external {
        rvm.setNonce(TARGET, 333);
        rvm.warp(12345);
        rvm.roll(67890);
    }

    function nonce_and_warp_roll() external view returns (bool) {
        return rvm.getNonce(TARGET) == 333
            && block.timestamp == 12345
            && block.number == 67890;
    }

    // --- Self-setNonce overwrites setup ---

    function call_self_set_nonce(uint64 nonce) external {
        rvm.setNonce(address(this), nonce);
    }

    function self_set_nonce_overwrites_setup() external view returns (bool) {
        return rvm.getNonce(address(this)) == 50;
    }
}
