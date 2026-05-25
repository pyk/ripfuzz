// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "./Vm.sol";

/// @notice Minimal stateful-fuzzing target for raptor prevrandao cheatcode.
///
/// Setup establishes a canonical `block.prevrandao` via `vm.prevrandao`.
/// Actions mutate or restore the value; invariants verify the canonical
/// state is intact.
contract PrevrandaoTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    bytes32 constant CANONICAL =
        bytes32(
            uint256(
                0x4242424242424242424242424242424242424242424242424242424242424242
            )
        );

    uint256 public storedPrevrandao;

    function setup() external {
        vm.prevrandao(CANONICAL);
        storedPrevrandao = block.prevrandao;
    }

    /// Re-set the canonical prevrandao and store it.
    function actionRestoreCanonical() external {
        vm.prevrandao(CANONICAL);
        storedPrevrandao = block.prevrandao;
    }

    /// Set a non-canonical prevrandao and store it.
    function actionMutateValue() external {
        vm.prevrandao(bytes32(uint256(0xdeadbeef)));
        storedPrevrandao = block.prevrandao;
    }

    /// Interleave multiple prevrandao values, ending on the canonical one.
    function actionSequence() external {
        vm.prevrandao(bytes32(uint256(1)));
        vm.prevrandao(bytes32(uint256(2)));
        vm.prevrandao(CANONICAL);
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
