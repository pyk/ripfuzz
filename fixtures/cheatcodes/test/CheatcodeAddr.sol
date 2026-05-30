// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAddr {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    address public derivedFromSetup;
    address public derivedFromCall;
    address public lastStoredAddr;

    // Known test vectors
    address public constant ADDR_PK_1 = address(0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);

    // --- setup interaction ---

    function setup() external {
        derivedFromSetup = vm.addr(1);
    }

    function setup_addr_persists() external view returns (bool) {
        return derivedFromSetup == ADDR_PK_1;
    }

    // --- Same-sequence visibility ---

    function call_derive_and_store(uint256 pk) external {
        // We use a fixed pk (100) in the test sequence so the property is deterministic.
        derivedFromCall = vm.addr(pk);
    }

    function addr_visible_in_next_call() external view returns (bool) {
        return derivedFromCall == address(0xd9A284367b6D3e25A91c91b5A430AF2593886EB9);
    }

    // --- Revert safety (contract stores addr, then reverts) ---

    function call_derive_and_revert(uint256 pk) external {
        lastStoredAddr = vm.addr(pk);
        revert("intentional");
    }

    function revert_undoes_storage() external view returns (bool) {
        // If call_derive_and_revert reverted, lastStoredAddr must still be address(0)
        return lastStoredAddr == address(0);
    }

    // --- Overwrite within same sequence ---

    function call_store_pk_1() external {
        lastStoredAddr = vm.addr(1);
    }

    function call_store_pk_2() external {
        lastStoredAddr = vm.addr(2);
    }

    function last_addr_overwrite() external view returns (bool) {
        return lastStoredAddr == address(0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF);
    }

    // --- Edge: invalid private key = 0 ---

    function call_addr_zero() external {
        lastStoredAddr = vm.addr(0);
    }

    function addr_zero_reverts() external view returns (bool) {
        // If the call reverted correctly, lastStoredAddr must still be address(0)
        return lastStoredAddr == address(0);
    }

    // --- Edge: invalid private key >= curve order ---

    function call_addr_too_large() external {
        // secp256k1_order = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
        lastStoredAddr = vm.addr(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141);
    }

    function addr_too_large_reverts() external view returns (bool) {
        return lastStoredAddr == address(0);
    }

    // --- Edge: boundary valid key (order - 1) ---

    function call_addr_boundary() external {
        lastStoredAddr = vm.addr(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140);
    }

    function addr_boundary_ok() external view returns (bool) {
        // The address for order-1 is a known test vector we can hardcode after verifying once.
        return lastStoredAddr == address(0x80C0dbf239224071c59dD8970ab9d542E3414aB2);
    }

    // --- Property sees final stored address ---

    function final_addr() external view returns (bool) {
        return lastStoredAddr == ADDR_PK_1;
    }

    // --- No interference with block cheatcodes ---

    function call_addr_and_warp_roll(uint256 pk) external {
        lastStoredAddr = vm.addr(pk);
        vm.warp(12345);
        vm.roll(67890);
    }

    function addr_and_warp_roll() external view returns (bool) {
        return lastStoredAddr == ADDR_PK_1
            && block.timestamp == 12345
            && block.number == 67890;
    }
}
