// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz sign cheatcode.
///
/// Setup derives well-known signatures via `rvm.sign` and stores them.
/// Actions re-derive signatures; invariants verify they recover to the
/// correct addresses.
contract SignHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    bytes32 constant DIGEST = 0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;

    uint256 constant MAX_VALID_KEY = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140;

    address constant ADDR_ONE = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;
    address constant ADDR_TWO = 0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF;
    address constant ADDR_MAX = 0x80C0dbf239224071c59dD8970ab9d542E3414aB2;

    uint8 public vOne;
    bytes32 public rOne;
    bytes32 public sOne;

    uint8 public vTwo;
    bytes32 public rTwo;
    bytes32 public sTwo;

    uint8 public vMax;
    bytes32 public rMax;
    bytes32 public sMax;

    function setup() external {
        (vOne, rOne, sOne) = rvm.sign(1, DIGEST);
        (vTwo, rTwo, sTwo) = rvm.sign(2, DIGEST);
        (vMax, rMax, sMax) = rvm.sign(MAX_VALID_KEY, DIGEST);
    }

    /// Re-sign with key 1 and store it.
    function actionResignOne() external {
        (vOne, rOne, sOne) = rvm.sign(1, DIGEST);
    }

    /// Re-sign with key 2 and store it.
    function actionResignTwo() external {
        (vTwo, rTwo, sTwo) = rvm.sign(2, DIGEST);
    }

    /// Re-sign with the max valid key and store it.
    function actionResignMaxValid() external {
        (vMax, rMax, sMax) = rvm.sign(MAX_VALID_KEY, DIGEST);
    }

    /// rvm.sign(0) must revert.
    function actionSignZero() external pure {
        rvm.sign(0, DIGEST);
    }

    /// rvm.sign with key >= curve order must revert.
    function actionSignOrder() external pure {
        rvm.sign(MAX_VALID_KEY + 1, DIGEST);
    }

    /// Use rvm.addr and rvm.sign together and return both addresses.
    function actionSignAndAddr() external pure returns (address derived, address recovered) {
        derived = rvm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = rvm.sign(1, DIGEST);
        recovered = ecrecover(DIGEST, v, r, s);
    }

    /// Invariant: signature from key 1 must recover to ADDR_ONE.
    function invariant_sigOneValid() external view {
        assert(ecrecover(DIGEST, vOne, rOne, sOne) == ADDR_ONE);
    }

    /// Invariant: signature from key 2 must recover to ADDR_TWO.
    function invariant_sigTwoValid() external view {
        assert(ecrecover(DIGEST, vTwo, rTwo, sTwo) == ADDR_TWO);
    }

    /// Invariant: signature from max valid key must recover to ADDR_MAX.
    function invariant_sigMaxValid() external view {
        assert(ecrecover(DIGEST, vMax, rMax, sMax) == ADDR_MAX);
    }
}
