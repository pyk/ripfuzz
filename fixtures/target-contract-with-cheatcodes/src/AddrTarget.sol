// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title AddrTarget
/// @notice Real-world fuzzing target that derives actor addresses from private
///         keys during setup and re-derives them in actions. Invariants verify
///         that `vm.addr` remains deterministic across the campaign.
contract AddrTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    /// Largest valid secp256k1 private key (curve order - 1).
    uint256 constant MAX_VALID_KEY =
        0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140;

    address public admin;
    address public voter;
    address public proposer;

    function setup() external {
        admin = vm.addr(1);
        voter = vm.addr(2);
        proposer = vm.addr(MAX_VALID_KEY);
    }

    /// Invariant: all stored actor addresses must match the well-known
    /// addresses derived from their respective private keys.
    function invariant_actorsMatch() external view {
        assert(admin == 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);
        assert(voter == 0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF);
        assert(proposer == 0x80C0dbf239224071c59dD8970ab9d542E3414aB2);
    }

    /// Action: re-derive admin address and overwrite storage.
    /// Fuzzer uses this to prove `vm.addr(1)` is deterministic across txs.
    function actionRefreshAdmin() external {
        admin = vm.addr(1);
    }

    /// Action: re-derive voter address and overwrite storage.
    function actionRefreshVoter() external {
        voter = vm.addr(2);
    }

    /// Action: re-derive proposer address and overwrite storage.
    function actionRefreshProposer() external {
        proposer = vm.addr(MAX_VALID_KEY);
    }

    /// Action: re-derive all actor addresses in one transaction.
    function actionRefreshAll() external {
        admin = vm.addr(1);
        voter = vm.addr(2);
        proposer = vm.addr(MAX_VALID_KEY);
    }

    /// Action: interleave different keys to prove no internal corruption.
    function actionRefreshInterleaved() external {
        address a = vm.addr(1);
        address b = vm.addr(2);
        address c = vm.addr(1);
        address d = vm.addr(MAX_VALID_KEY);
        admin = a;
        voter = b;
        proposer = d;
        assert(c == a);
    }

    /// Action: call `vm.addr(0)` which must revert.
    function actionInvalidZero() external pure {
        vm.addr(0);
    }

    /// Action: call `vm.addr` with a key >= curve order which must revert.
    function actionInvalidOrder() external pure {
        vm.addr(MAX_VALID_KEY + 1);
    }
}
