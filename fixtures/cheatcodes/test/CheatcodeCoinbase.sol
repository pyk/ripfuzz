// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeCoinbase {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    address public recordedCoinbase;
    uint256 public recordedBlockNumber;
    uint256 public recordedTimestamp;
    uint256 public recordedBaseFee;

    // --- setUp interaction ---

    function setUp() external {
        vm.coinbase(address(0xCA11BA5E));
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

    function property_setup_coinbase_persists() external view returns (bool) {
        return recordedCoinbase == address(0xCA11BA5E);
    }

    function property_setup_only() external view returns (bool) {
        return block.coinbase == address(0xCA11BA5E);
    }

    // --- Same-sequence persistence ---

    function call_coinbase(address addr) external {
        vm.coinbase(addr);
        recordedCoinbase = block.coinbase;
    }

    function property_coinbase_persists_across_calls() external view returns (bool) {
        // call_coinbase(0xAB) -> coinbase = 0xAB, next call sees 0xAB (no auto-advance)
        return recordedCoinbase == address(0xAB);
    }

    // --- Revert safety ---

    function call_coinbase_and_revert(address addr) external {
        vm.coinbase(addr);
        revert("intentional");
    }

    function property_revert_undoes_coinbase() external view returns (bool) {
        return block.coinbase != address(0xDEAD);
    }

    // --- Coinbase overwrite ---

    function call_coinbase_A() external {
        vm.coinbase(address(0xA));
    }

    function call_coinbase_B() external {
        vm.coinbase(address(0xB));
    }

    function property_coinbase_overwrite() external view returns (bool) {
        // call_coinbase_A -> 0xA, call_coinbase_B -> 0xB
        return block.coinbase == address(0xB);
    }

    // --- Edge: coinbase to zero address ---

    function call_coinbase_zero() external {
        vm.coinbase(address(0));
    }

    function property_coinbase_zero() external view returns (bool) {
        return block.coinbase == address(0);
    }

    // --- Property sees final coinbase ---

    function property_final_coinbase() external view returns (bool) {
        // If the only call was call_coinbase_A(), the property should see 0xA
        return block.coinbase == address(0xA);
    }

    // --- Cross-cheatcode interaction: coinbase + roll + warp + fee ---

    function call_coinbase_and_roll_warp_fee() external {
        vm.coinbase(address(0xC011B4a5E0000000000000000000000000000000));
        vm.roll(7000);
        vm.warp(9000);
        vm.fee(5000);
    }

    function property_coinbase_and_roll_warp_fee() external view returns (bool) {
        return block.coinbase == address(0xC011B4a5E0000000000000000000000000000000)
            && block.number == 7000
            && block.timestamp == 9000
            && block.basefee == 5000;
    }
}
