// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {RVM} from "../src/RVM.sol";

contract CheatcodePrevrandao {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));
    bytes32 public recordedPrevrandao;
    uint256 public recordedBlockNumber;
    uint256 public recordedTimestamp;
    uint256 public recordedBaseFee;
    address public recordedCoinbase;
    uint256 public recordedDifficulty;

    // --- setup interaction ---

    function setup() external {
        rvm.prevrandao(bytes32(uint256(0xCA11BA5E)));
    }

    function call_record_prevrandao() external {
        recordedPrevrandao = bytes32(uint256(block.prevrandao));
    }

    function call_record_block_number() external {
        recordedBlockNumber = block.number;
    }

    function call_record_timestamp() external {
        recordedTimestamp = block.timestamp;
    }

    function call_record_basefee() external {
        recordedBaseFee = block.basefee;
    }

    function call_record_coinbase() external {
        recordedCoinbase = block.coinbase;
    }

    function setup_prevrandao_persists() external view returns (bool) {
        return recordedPrevrandao == bytes32(uint256(0xCA11BA5E));
    }

    function setup_only() external view returns (bool) {
        return block.prevrandao == uint256(0xCA11BA5E);
    }

    // --- Same-sequence persistence ---

    function call_prevrandao(bytes32 val) external {
        rvm.prevrandao(val);
        recordedPrevrandao = bytes32(uint256(block.prevrandao));
    }

    function prevrandao_persists_across_calls() external view returns (bool) {
        // call_prevrandao(0xAB) -> prevrandao = 0xAB, next call sees 0xAB (no auto-advance)
        return recordedPrevrandao == bytes32(uint256(0xAB));
    }

    // --- Revert safety ---

    function call_prevrandao_and_revert(bytes32 val) external {
        rvm.prevrandao(val);
        revert("intentional");
    }

    function revert_undoes_prevrandao() external view returns (bool) {
        return block.prevrandao != uint256(0xDEAD);
    }

    // --- Prevrandao overwrite ---

    function call_prevrandao_A() external {
        rvm.prevrandao(bytes32(uint256(0xA)));
    }

    function call_prevrandao_B() external {
        rvm.prevrandao(bytes32(uint256(0xB)));
    }

    function prevrandao_overwrite() external view returns (bool) {
        // call_prevrandao_A -> 0xA, call_prevrandao_B -> 0xB
        return block.prevrandao == uint256(0xB);
    }

    // --- Edge: prevrandao to zero bytes32 ---

    function call_prevrandao_zero() external {
        rvm.prevrandao(bytes32(0));
    }

    function prevrandao_zero() external view returns (bool) {
        return block.prevrandao == uint256(0);
    }

    // --- Edge: prevrandao to max uint256 ---

    function call_prevrandao_max() external {
        rvm.prevrandao(bytes32(type(uint256).max));
    }

    function prevrandao_max() external view returns (bool) {
        return block.prevrandao == uint256(type(uint256).max);
    }

    // --- Property sees final prevrandao ---

    function final_prevrandao() external view returns (bool) {
        // If the only call was call_prevrandao_A(), the property should see 0xA
        return block.prevrandao == uint256(0xA);
    }

    // --- Cross-cheatcode interaction: prevrandao + roll + warp + fee + coinbase ---

    function call_prevrandao_and_roll_warp_fee_coinbase() external {
        rvm.prevrandao(bytes32(uint256(0xBEEF)));
        rvm.roll(7000);
        rvm.warp(9000);
        rvm.fee(5000);
        rvm.coinbase(address(0xC011BA5E));
    }

    function prevrandao_and_roll_warp_fee_coinbase() external view returns (bool) {
        return block.prevrandao == uint256(0xBEEF)
            && block.number == 7000
            && block.timestamp == 9000
            && block.basefee == 5000
            && block.coinbase == address(0xC011BA5E);
    }

    // --- Interaction with difficulty no-op ---

    function call_prevrandao_then_difficulty() external {
        rvm.prevrandao(bytes32(uint256(0xF00D)));
        rvm.difficulty(9999);
        recordedDifficulty = block.difficulty;
        recordedPrevrandao = bytes32(uint256(block.prevrandao));
    }

    function difficulty_noop_does_not_clobber() external view returns (bool) {
        // On post-Paris, block.difficulty reads prevrandao.
        // rvm.difficulty(9999) is a no-op and must not overwrite the prior prevrandao.
        return recordedDifficulty == uint256(0xF00D)
            && recordedPrevrandao == bytes32(uint256(0xF00D));
    }
}
