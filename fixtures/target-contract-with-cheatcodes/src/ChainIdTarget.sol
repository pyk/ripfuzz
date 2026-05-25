// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract ChainIdTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_CHAIN_ID = 42;

    uint256 public storedChainId;

    function setup() external {
        vm.chainId(EXPECTED_CHAIN_ID);
        storedChainId = block.chainid;
    }

    function getChainId() external view returns (uint256) {
        return block.chainid;
    }

    function getStoredChainId() external view returns (uint256) {
        return storedChainId;
    }

    /// Call vm.chainId with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callChainIdSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.chainId(EXPECTED_CHAIN_ID);
        first = block.chainid;
        vm.chainId(EXPECTED_CHAIN_ID);
        second = block.chainid;
    }

    /// Call vm.chainId with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callChainIdSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.chainId(1);
        first = block.chainid;
        vm.chainId(EXPECTED_CHAIN_ID);
        second = block.chainid;
        vm.chainId(1);
        third = block.chainid;
    }

    /// Fuzzing action: re-set the chain id and store it.
    function actionChainId() external {
        vm.chainId(EXPECTED_CHAIN_ID);
        storedChainId = block.chainid;
    }

    function invariant_chain_id() external view {
        assert(storedChainId == EXPECTED_CHAIN_ID);
    }
}
