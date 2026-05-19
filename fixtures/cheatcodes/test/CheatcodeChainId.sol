// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeChainId {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));
    uint256 public recordedChainId;
    uint256 public recordedTimestamp;

    // --- setUp interaction ---

    function setUp() external {
        vm.chainId(1337);
    }

    function call_record() external {
        recordedChainId = block.chainid;
    }

    function setup_chain_id_persists() external view returns (bool) {
        return recordedChainId == 1337;
    }

    function setup_only() external view returns (bool) {
        return block.chainid == 1337;
    }

    // --- Same-sequence persistence ---

    function call_chain_id(uint256 id) external {
        vm.chainId(id);
        recordedChainId = block.chainid;
    }

    function chain_id_persists() external view returns (bool) {
        return recordedChainId == 9999;
    }

    // --- Revert safety ---

    function call_chain_id_and_revert(uint256 id) external {
        vm.chainId(id);
        revert("intentional");
    }

    function revert_undoes_chain_id() external view returns (bool) {
        return block.chainid == 1337;
    }

    // --- Overwrite ---

    function call_chain_id_100() external {
        vm.chainId(100);
    }

    function call_chain_id_200() external {
        vm.chainId(200);
    }

    function chain_id_overwrite() external view returns (bool) {
        return recordedChainId == 200;
    }

    // --- Edge: chainId to zero ---

    function call_chain_id_zero() external {
        vm.chainId(0);
    }

    function chain_id_zero() external view returns (bool) {
        return recordedChainId == 0;
    }

    // --- Edge: chainId to max uint64 ---

    function call_chain_id_max_u64() external {
        vm.chainId(type(uint64).max);
    }

    function chain_id_max_u64() external view returns (bool) {
        return recordedChainId == type(uint64).max;
    }

    // --- Edge: chainId too large ---

    function call_chain_id_too_large() external {
        vm.chainId(uint256(type(uint64).max) + 1);
    }

    function chain_id_too_large_reverts() external view returns (bool) {
        // If the cheatcode reverted correctly, the chain ID must be unchanged.
        return block.chainid == 1337;
    }

    // --- Property sees final chainId ---

    function final_chain_id() external view returns (bool) {
        // If the only call was call_chain_id_100(), the property should see 100
        return block.chainid == 100;
    }

    // --- Cross-cheatcode interaction: chainId + warp ---

    function call_chain_id_and_warp() external {
        vm.chainId(12345);
        vm.warp(67890);
        recordedChainId = block.chainid;
        recordedTimestamp = block.timestamp;
    }

    function chain_id_and_warp() external view returns (bool) {
        return recordedChainId == 12345 && recordedTimestamp == 67890;
    }
}
