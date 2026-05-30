// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @notice Minimal stateful-fuzzing target for raptor warp cheatcode.
///
/// Setup warps to a canonical timestamp and stores it.
/// Actions restore or mutate the timestamp; invariants verify the value.
contract WarpTarget {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant EXPECTED = 1_234_567_890;

    uint256 public storedTimestamp;

    function setup() external {
        vm.warp(EXPECTED);
        storedTimestamp = block.timestamp;
    }

    /// Read block.timestamp to prove persistence after deploy/setup.
    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }

    /// Re-warp to the canonical value and store it.
    function actionWarp() external {
        vm.warp(EXPECTED);
        storedTimestamp = block.timestamp;
    }

    /// Warp to a non-canonical value and store it.
    function actionMutate() external {
        vm.warp(99);
        storedTimestamp = block.timestamp;
    }

    function invariant_warp() external view {
        assert(block.timestamp == EXPECTED);
    }
}
