// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @notice Exercises `rvm.fork` for single- and multi-fork campaigns.
///
/// Includes helpers for same-address / different-chain isolation tests
/// (e.g. a bridge contract deployed at the same address on Ethereum and Polygon)
/// and for tracking local harness state across forks (value conservation).
contract ForkHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Shared remote address used for isolation tests (PolyBridger-style).
    address constant BRIDGE = 0x1111111111111111111111111111111111111111;

    string public lastUrl;
    uint256 public lastBlock;
    uint256 public lastChainId;
    uint256 public lastTimestamp;
    bytes32 public lastSlot0;
    uint256 public lastBalance;

    /// Ghost / local state that must survive fork switches so campaigns can
    /// track value conservation across chains (e.g. amount locked on L1 vs
    /// minted on L2).
    uint256 public trackedValue;
    uint256 public totalOutflow;
    uint256 public totalInflow;

    function setup() external {
        // Default setup stays on the empty sandbox so tests can drive forks
        // explicitly from actions.
        lastChainId = block.chainid;
        lastTimestamp = block.timestamp;
        lastBlock = block.number;
        trackedValue = 0;
        totalOutflow = 0;
        totalInflow = 0;
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

    /// Store on fork A, then switch to fork B inside the same transaction.
    ///
    /// The store is visible before the switch (`lastSlot0`) and must also
    /// persist on fork A after later selecting A again.
    function actionForkStoreThenSwitch(
        string calldata urlA,
        uint256 blockA,
        bytes32 value,
        string calldata urlB,
        uint256 blockB
    ) external {
        rvm.fork(urlA, blockA);
        rvm.store(BRIDGE, bytes32(uint256(0)), value);
        // Same-tx check: journaled write is visible before switching forks.
        lastSlot0 = rvm.load(BRIDGE, bytes32(uint256(0)));
        rvm.fork(urlB, blockB);
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Deal on fork A, then switch to fork B inside the same transaction.
    ///
    /// Same mid-tx persistence guarantee as `actionForkStoreThenSwitch`.
    function actionForkDealThenSwitch(
        string calldata urlA,
        uint256 blockA,
        uint256 value,
        string calldata urlB,
        uint256 blockB
    ) external {
        rvm.fork(urlA, blockA);
        rvm.deal(BRIDGE, value);
        lastBalance = BRIDGE.balance;
        rvm.fork(urlB, blockB);
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Read bridge slot 0 on the currently active fork (no re-fork).
    function actionReadBridge() external {
        lastSlot0 = rvm.load(BRIDGE, bytes32(uint256(0)));
        lastBalance = BRIDGE.balance;
    }

    /// Set local ghost state without forking.
    function actionSetTracked(uint256 value) external {
        trackedValue = value;
    }

    /// Fork, then bump local tracked value. Harness storage must persist when
    /// later switching to another fork.
    function actionForkAndBumpTracked(string calldata url, uint256 blockNumber, uint256 delta) external {
        rvm.fork(url, blockNumber);
        trackedValue += delta;
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Record an outflow on the source chain (e.g. lock / burn), then keep the
    /// cumulative total in harness storage for cross-chain conservation checks.
    function actionRecordOutflow(string calldata url, uint256 blockNumber, uint256 amount) external {
        rvm.fork(url, blockNumber);
        totalOutflow += amount;
        trackedValue += amount;
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Record an inflow on the destination chain (e.g. mint / unlock). Local
    /// totals from the source chain must still be visible after the fork switch.
    function actionRecordInflow(string calldata url, uint256 blockNumber, uint256 amount) external {
        rvm.fork(url, blockNumber);
        totalInflow += amount;
        lastBlock = block.number;
        lastChainId = block.chainid;
    }

    /// Value conservation: every outflow should eventually match inflow.
    function invariant_conservation() external view {
        assert(totalOutflow == totalInflow);
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

    function getTrackedValue() external view returns (uint256) {
        return trackedValue;
    }

    function getTotalOutflow() external view returns (uint256) {
        return totalOutflow;
    }

    function getTotalInflow() external view returns (uint256) {
        return totalInflow;
    }
}
