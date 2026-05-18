// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeSign {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    // Known test vectors
    address public constant ADDR_PK_1 = address(0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);
    address public constant ADDR_PK_2 = address(0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF);
    uint256 public constant PK_1 = 1;
    bytes32 public constant DIGEST = keccak256("Data To Sign");

    // Storage for signature components
    uint8 public storedV;
    bytes32 public storedR;
    bytes32 public storedS;
    address public recoveredAddr;

    // --- setUp interaction ---

    function setUp() external {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PK_1, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
    }

    function property_setup_sign_persists() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1;
    }

    function property_setup_only() external view returns (bool) {
        // After setUp, the signature should already be stored.
        return recoveredAddr == ADDR_PK_1;
    }

    // --- Same-sequence visibility ---

    function call_sign_and_store(uint256 pk) external {
        // Test uses pk = 1 so the property is deterministic.
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
    }

    function property_sign_visible_in_next_call() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1;
    }

    // --- Revert safety (contract stores sig, then reverts) ---

    function call_sign_and_revert(uint256 pk) external {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
        revert("intentional");
    }

    function property_revert_undoes_storage() external view returns (bool) {
        // If call_sign_and_revert reverted, all storage must be back to setUp values.
        return recoveredAddr == ADDR_PK_1;
    }

    // --- Overwrite within same sequence ---

    function call_sign_pk_1() external {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PK_1, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
    }

    function call_sign_pk_2() external {
        uint256 pk2 = 2;
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk2, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
    }

    function property_last_sign_overwrite() external view returns (bool) {
        return recoveredAddr == ADDR_PK_2;
    }

    // --- Edge: invalid private key = 0 ---

    function call_sign_zero() external {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(0, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
    }

    function property_sign_zero_reverts() external view returns (bool) {
        // If the call reverted correctly, storage must still hold setUp values.
        return recoveredAddr == ADDR_PK_1;
    }

    // --- Edge: invalid private key >= curve order ---

    function call_sign_too_large() external {
        uint256 order = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(order, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
    }

    function property_sign_too_large_reverts() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1;
    }

    // --- Edge: boundary valid key (order - 1) ---

    function call_sign_boundary() external {
        uint256 orderMinus1 = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140;
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(orderMinus1, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
    }

    function property_sign_boundary_ok() external view returns (bool) {
        return recoveredAddr != address(0);
    }

    // --- Property sees final stored signature ---

    function property_final_signature() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1;
    }

    // --- No interference with block cheatcodes ---

    function call_sign_and_warp_roll(uint256 pk) external {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, DIGEST);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(DIGEST, v, r, s);
        vm.warp(12345);
        vm.roll(67890);
    }

    function property_sign_and_warp_roll() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1
            && block.timestamp == 12345
            && block.number == 67890;
    }

    // --- Ecrecover compatibility with a different digest ---

    function call_sign_different_digest(uint256 pk) external {
        bytes32 digest2 = keccak256("Another Digest");
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest2);
        storedV = v;
        storedR = r;
        storedS = s;
        recoveredAddr = ecrecover(digest2, v, r, s);
    }

    function property_different_digest_recoverable() external view returns (bool) {
        return recoveredAddr == ADDR_PK_1;
    }
}
