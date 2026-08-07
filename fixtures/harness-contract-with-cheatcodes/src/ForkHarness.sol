// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @notice Exercises `rvm.fork` for single- and multi-fork campaigns.
///
/// Includes helpers for same-address / different-chain isolation tests
/// (e.g. a bridge contract deployed at the same address on Ethereum and Polygon).
contract ForkHarness {
    RVM constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Shared remote address used for isolation tests (PolyBridger-style).
    address constant BRIDGE = 0x1111111111111111111111111111111111111111;

    string public lastUrl;
    uint256 public lastBlock;
    uint256 public lastChainId;
    uint256 public lastTimestamp;
    bytes32 public lastSlot0;
    uint256 public lastBalance;

    function setup() external {
        // Default setup stays on the empty sandbox so tests can drive forks
        // explicitly from actions.
        lastChainId = block.chainid;
        lastTimestamp = block.timestamp;
        lastBlock = block.number;
    }

    function actionFork(string calldata url, uint256 blockNumber) external {
        rvm.fork(url, blockNumber);
        lastUrl = url;
        lastBlock = block.number;
        lastChainId = block.chainid;
        lastTimestamp = block.timestamp;
    }

    function actionForkWithConfig(
        string calldata url,
        uint256 blockNumber,
        uint32 retries,
        uint64 backoffMs,
        uint64 timeoutMs,
        uint64 rateLimit
    ) external {
        RVM.ForkConfig memory config = RVM.ForkConfig({
            retries: retries, backoffMs: backoffMs, timeoutMs: timeoutMs, rateLimit: rateLimit
        });
        rvm.fork(url, blockNumber, config);
        lastUrl = url;
        lastBlock = block.number;
        lastChainId = block.chainid;
        lastTimestamp = block.timestamp;
    }

    /// Fork then read remote bridge slot 0 and balance.
    function actionForkAndReadBridge(string calldata url, uint256 blockNumber) external {
        rvm.fork(url, blockNumber);
        lastSlot0 = rvm.load(BRIDGE, bytes32(uint256(0)));
        lastBalance = BRIDGE.balance;
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Fork, mutate bridge storage via rvm.store, then re-read.
    function actionForkStoreBridge(string calldata url, uint256 blockNumber, bytes32 value) external {
        rvm.fork(url, blockNumber);
        rvm.store(BRIDGE, bytes32(uint256(0)), value);
        lastSlot0 = rvm.load(BRIDGE, bytes32(uint256(0)));
    }

    /// Fork, mutate bridge balance via rvm.deal, then re-read.
    function actionForkDealBridge(string calldata url, uint256 blockNumber, uint256 value) external {
        rvm.fork(url, blockNumber);
        rvm.deal(BRIDGE, value);
        lastBalance = BRIDGE.balance;
    }

    /// Read bridge slot 0 on the currently active fork (no re-fork).
    function actionReadBridge() external {
        lastSlot0 = rvm.load(BRIDGE, bytes32(uint256(0)));
        lastBalance = BRIDGE.balance;
    }

    function getBlockNumber() external view returns (uint256) {
        return block.number;
    }

    function getChainId() external view returns (uint256) {
        return block.chainid;
    }

    function getTimestamp() external view returns (uint256) {
        return block.timestamp;
    }

    function getLastSlot0() external view returns (bytes32) {
        return lastSlot0;
    }

    function getLastBalance() external view returns (uint256) {
        return lastBalance;
    }
}
