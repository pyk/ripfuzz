// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "./RVM.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz prevrandao cheatcode.
///
/// Setup establishes a canonical `block.prevrandao` via `rvm.prevrandao`.
/// Actions mutate or restore the value; invariants verify the canonical
/// state is intact.
contract PrevrandaoHarness {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    bytes32 constant CANONICAL = bytes32(uint256(0x4242424242424242424242424242424242424242424242424242424242424242));

    uint256 public storedPrevrandao;

    function setup() external {
        rvm.prevrandao(CANONICAL);
        storedPrevrandao = block.prevrandao;
    }

    /// Re-set the canonical prevrandao and store it.
    function actionRestoreCanonical() external {
        rvm.prevrandao(CANONICAL);
        storedPrevrandao = block.prevrandao;
    }

    /// Set a non-canonical prevrandao and store it.
    function actionMutateValue() external {
        rvm.prevrandao(bytes32(uint256(0xdeadbeef)));
        storedPrevrandao = block.prevrandao;
    }

    /// Interleave multiple prevrandao values, ending on the canonical one.
    function actionSequence() external {
        rvm.prevrandao(bytes32(uint256(1)));
        rvm.prevrandao(bytes32(uint256(2)));
        rvm.prevrandao(CANONICAL);
        storedPrevrandao = block.prevrandao;
    }

    /// Read block.prevrandao without calling any cheatcode.  Proves the
    /// value set during setup persists across the exec via block_env.
    function actionReadPrevrandao() external {
        storedPrevrandao = block.prevrandao;
    }

    /// Directly return the current `block.prevrandao`.
    function getPrevrandao() external view returns (uint256) {
        return block.prevrandao;
    }

    /// Read the stored prevrandao value.
    function getStoredPrevrandao() external view returns (uint256) {
        return storedPrevrandao;
    }

    /// Invariant: stored prevrandao must match the canonical value.
    function invariant_prevrandaoMatch() external view {
        assert(storedPrevrandao == uint256(CANONICAL));
    }
}
