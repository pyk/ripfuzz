// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title ChainIdHandler
/// @notice Real-world fuzz handler that controls `block.chainid` via the
///         `vm.chainId` cheatcode. Setup establishes a canonical chain id and
///         actions mutate or restore it. Invariants verify deterministic control.
contract ChainIdHandler {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant EXPECTED_CHAIN_ID = 42;

    function setup() external {
        vm.chainId(EXPECTED_CHAIN_ID);
    }

    /// Invariant: the live chain id must always match the expected value.
    function invariant_chainId() external view {
        assert(block.chainid == EXPECTED_CHAIN_ID);
    }

    /// Action: re-set the chain id to the expected value.
    function actionRestoreChainId() external {
        vm.chainId(EXPECTED_CHAIN_ID);
    }

    /// Action: temporarily set a different chain id.
    function actionMutateChainId() external {
        vm.chainId(1337);
    }

    /// Action: interleave chain id changes inside one tx, ending on expected.
    function actionChainIdSequence() external {
        vm.chainId(1);
        vm.chainId(EXPECTED_CHAIN_ID);
        vm.chainId(999);
        vm.chainId(EXPECTED_CHAIN_ID);
    }
}
