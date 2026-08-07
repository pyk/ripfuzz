// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeCoinbase {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));
    address public recordedCoinbase;
    uint256 public recordedBlockNumber;
    uint256 public recordedTimestamp;
    uint256 public recordedBaseFee;

    // --- setup interaction ---

    function setup() external {
        rvm.coinbase(address(0xCA11BA5E));
    }

    function call_record_coinbase() external {
        recordedCoinbase = block.coinbase;
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

    function setup_coinbase_persists() external view returns (bool) {
        return recordedCoinbase == address(0xCA11BA5E);
    }

    function setup_only() external view returns (bool) {
        return block.coinbase == address(0xCA11BA5E);
    }

    // --- Same-sequence persistence ---

    function call_coinbase(address addr) external {
        rvm.coinbase(addr);
        recordedCoinbase = block.coinbase;
    }

    function coinbase_persists_across_calls() external view returns (bool) {
        // call_coinbase(0xAB) -> coinbase = 0xAB, next call sees 0xAB (no auto-advance)
        return recordedCoinbase == address(0xAB);
    }

    // --- Revert safety ---

    function call_coinbase_and_revert(address addr) external {
        rvm.coinbase(addr);
        revert("intentional");
    }

    function revert_undoes_coinbase() external view returns (bool) {
        return block.coinbase != address(0xDEAD);
    }

    // --- Coinbase overwrite ---

    function call_coinbase_A() external {
        rvm.coinbase(address(0xA));
    }

    function call_coinbase_B() external {
        rvm.coinbase(address(0xB));
    }

    function coinbase_overwrite() external view returns (bool) {
        // call_coinbase_A -> 0xA, call_coinbase_B -> 0xB
        return block.coinbase == address(0xB);
    }

    // --- Edge: coinbase to zero address ---

    function call_coinbase_zero() external {
        rvm.coinbase(address(0));
    }

    function coinbase_zero() external view returns (bool) {
        return block.coinbase == address(0);
    }

    // --- Property sees final coinbase ---

    function final_coinbase() external view returns (bool) {
        // If the only call was call_coinbase_A(), the property should see 0xA
        return block.coinbase == address(0xA);
    }

    // --- Cross-cheatcode interaction: coinbase + roll + warp + fee ---

    function call_coinbase_and_roll_warp_fee() external {
        rvm.coinbase(address(0xC011B4a5E0000000000000000000000000000000));
        rvm.roll(7000);
        rvm.warp(9000);
        rvm.fee(5000);
    }

    function coinbase_and_roll_warp_fee() external view returns (bool) {
        return block.coinbase == address(0xC011B4a5E0000000000000000000000000000000)
            && block.number == 7000
            && block.timestamp == 9000
            && block.basefee == 5000;
    }
}
