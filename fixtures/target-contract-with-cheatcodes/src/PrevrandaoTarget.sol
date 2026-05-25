// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "./Vm.sol";

contract PrevrandaoTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    bytes32 constant EXPECTED_PREVRANDAO =
        bytes32(
            uint256(
                0x4242424242424242424242424242424242424242424242424242424242424242
            )
        );

    uint256 public storedPrevrandao;

    function setup() external {
        vm.prevrandao(EXPECTED_PREVRANDAO);
        storedPrevrandao = block.prevrandao;
    }

    function getPrevrandao() external view returns (uint256) {
        return block.prevrandao;
    }

    function getStoredPrevrandao() external view returns (uint256) {
        return storedPrevrandao;
    }

    /// Call vm.prevrandao with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callPrevrandaoSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.prevrandao(EXPECTED_PREVRANDAO);
        first = block.prevrandao;
        vm.prevrandao(EXPECTED_PREVRANDAO);
        second = block.prevrandao;
    }

    /// Call vm.prevrandao with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callPrevrandaoSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.prevrandao(bytes32(uint256(1)));
        first = block.prevrandao;
        vm.prevrandao(EXPECTED_PREVRANDAO);
        second = block.prevrandao;
        vm.prevrandao(bytes32(uint256(2)));
        third = block.prevrandao;
    }

    /// Interaction with roll - both cheatcodes in same tx.
    function callPrevrandaoAndRoll()
        external
        returns (uint256 prevrandao, uint256 number)
    {
        vm.prevrandao(EXPECTED_PREVRANDAO);
        vm.roll(12345);
        prevrandao = block.prevrandao;
        number = block.number;
    }

    /// Fuzzing action: re-set the prevrandao and store it.
    function actionPrevrandao() external {
        vm.prevrandao(EXPECTED_PREVRANDAO);
        storedPrevrandao = block.prevrandao;
    }

    function invariant_prevrandao() external view {
        assert(storedPrevrandao == uint256(EXPECTED_PREVRANDAO));
    }
}
