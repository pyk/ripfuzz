// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @title ChainIdHarness
/// @notice Real-world fuzz handler that controls `block.chainid` via the
///         `rvm.chainId` cheatcode. Setup establishes a canonical chain id and
///         actions mutate or restore it. Invariants verify deterministic control.
contract ChainIdHarness {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant EXPECTED_CHAIN_ID = 42;

    function setup() external {
        rvm.chainId(EXPECTED_CHAIN_ID);
    }

    /// Invariant: the live chain id must always match the expected value.
    function invariant_chainId() external view {
        assert(block.chainid == EXPECTED_CHAIN_ID);
    }

    /// Action: re-set the chain id to the expected value.
    function actionRestoreChainId() external {
        rvm.chainId(EXPECTED_CHAIN_ID);
    }

    /// Action: temporarily set a different chain id.
    function actionMutateChainId() external {
        rvm.chainId(1337);
    }

    /// Action: interleave chain id changes inside one tx, ending on expected.
    function actionChainIdSequence() external {
        rvm.chainId(1);
        rvm.chainId(EXPECTED_CHAIN_ID);
        rvm.chainId(999);
        rvm.chainId(EXPECTED_CHAIN_ID);
    }
}
