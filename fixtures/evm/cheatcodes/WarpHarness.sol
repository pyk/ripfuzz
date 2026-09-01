// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";

/// @notice Minimal stateful-fuzz handler for ripfuzz warp cheatcode.
///
/// Setup warps to a canonical timestamp and stores it.
/// Actions restore or mutate the timestamp; invariants verify the value.
contract WarpHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    uint256 constant EXPECTED = 1_234_567_890;

    uint256 public storedTimestamp;

    function setup() external {
        rvm.warp(EXPECTED);
        storedTimestamp = block.timestamp;
    }

    /// Read block.timestamp to prove persistence after deploy/setup.
    function getBlockTimestamp() external view returns (uint256) {
        return block.timestamp;
    }

    /// Re-warp to the canonical value and store it.
    function actionWarp() external {
        rvm.warp(EXPECTED);
        storedTimestamp = block.timestamp;
    }

    /// Warp to a non-canonical value and store it.
    function actionMutate() external {
        rvm.warp(99);
        storedTimestamp = block.timestamp;
    }

    function invariant_warp() external view {
        assert(block.timestamp == EXPECTED);
    }
}
