// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./PrankVictim.sol";

/// @notice Integration-test target for raptor prank cheatcodes.
///
/// Covers `vm.prank`, `vm.startPrank`, `vm.stopPrank`, the `useActor`
/// modifier pattern, nested calls, constructor pranking, and
/// interactions with other cheatcodes.
contract PrankTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    PrankVictim public victim;
    PrankVictim public inner;

    address[] public actors;
    address public currentActor;

    // Storage used by invariants and cross-tx assertions.
    address public storedSender;
    address public storedOrigin;
    uint256 public storedBalance;
    uint256 public storedTimestamp;

    // Sequence checkpoints for single-tx determinism tests.
    address public seqSender1;
    address public seqSender2;
    address public seqSender3;
    address public seqSender4;
    address public seqOrigin2;

    // Well-known prank addresses (all non-zero so we can reject address(0)).
    address constant PRANK_ADDR = 0x1111111111111111111111111111111111111111;
    address constant PRANK_ADDR_2 = 0x2222222222222222222222222222222222222222;
    address constant PRANK_ORIGIN = 0x3333333333333333333333333333333333333333;
    address constant START_ADDR = 0x5555555555555555555555555555555555555555;
    address constant START_ORIGIN = 0x6666666666666666666666666666666666666666;
    address constant PERSIST_ADDR = 0x7777777777777777777777777777777777777777;
    address constant NESTED_ADDR = 0x9999999999999999999999999999999999999999;
    address constant CONSTRUCTOR_ADDR =
        0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC;

    modifier useActor(uint256 actorSeed) {
        currentActor = actors[actorSeed % actors.length];
        vm.startPrank(currentActor);
        _;
        vm.stopPrank();
    }

    function setup() external {
        victim = new PrankVictim();
        inner = new PrankVictim();
        actors = [
            address(0x1000000000000000000000000000000000000001),
            address(0x2000000000000000000000000000000000000002),
            address(0x3000000000000000000000000000000000000003)
        ];
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    // -----------------------------------------------------------------
    // Getters
    // -----------------------------------------------------------------

    function getVictimSender() external view returns (address) {
        return victim.lastSender();
    }

    function getVictimOrigin() external view returns (address) {
        return victim.lastOrigin();
    }

    function getInnerSender() external view returns (address) {
        return inner.lastSender();
    }

    function getInnerOrigin() external view returns (address) {
        return inner.lastOrigin();
    }

    function getCurrentActor() external view returns (address) {
        return currentActor;
    }

    function getSeqSender1() external view returns (address) {
        return seqSender1;
    }

    function getSeqSender2() external view returns (address) {
        return seqSender2;
    }

    function getSeqSender3() external view returns (address) {
        return seqSender3;
    }

    function getSeqSender4() external view returns (address) {
        return seqSender4;
    }

    function getSeqOrigin2() external view returns (address) {
        return seqOrigin2;
    }

    function getStoredSender() external view returns (address) {
        return storedSender;
    }

    function getStoredOrigin() external view returns (address) {
        return storedOrigin;
    }

    function getStoredBalance() external view returns (uint256) {
        return storedBalance;
    }

    function getStoredTimestamp() external view returns (uint256) {
        return storedTimestamp;
    }

    // -----------------------------------------------------------------
    // Basic prank
    // -----------------------------------------------------------------

    function callPrankSender() external {
        vm.prank(PRANK_ADDR);
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    function callPrankOrigin() external {
        vm.prank(PRANK_ADDR_2, PRANK_ORIGIN);
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    /// vm.prank is consumed by the very next call and cleaned up.
    function callPrankConsumed() external {
        vm.prank(PRANK_ADDR);
        victim.record();
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    // -----------------------------------------------------------------
    // startPrank / stopPrank
    // -----------------------------------------------------------------

    function callStartStop() external {
        vm.startPrank(START_ADDR, START_ORIGIN);
        victim.record();
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
        vm.stopPrank();
        victim.record();
    }

    function callStartNoStop() external {
        vm.startPrank(PERSIST_ADDR);
        victim.record();
    }

    function callAfterStartNoStop() external {
        victim.record();
    }

    function callAfterStop() external {
        vm.stopPrank();
        victim.record();
    }

    // -----------------------------------------------------------------
    // Overwrite validation (must revert)
    // -----------------------------------------------------------------

    function callDoublePrankReverts() external {
        vm.prank(PRANK_ADDR);
        vm.prank(PRANK_ADDR_2);
        victim.record();
    }

    function callStartOverwriteUnusedReverts() external {
        vm.startPrank(PRANK_ADDR);
        vm.startPrank(PRANK_ADDR_2);
        victim.record();
    }

    function callPrankOverStartReverts() external {
        vm.startPrank(PRANK_ADDR);
        vm.prank(PRANK_ADDR_2);
        victim.record();
    }

    // -----------------------------------------------------------------
    // Overwrite used startPrank (must succeed)
    // -----------------------------------------------------------------

    function callStartOverwriteUsed() external {
        vm.startPrank(PRANK_ADDR);
        victim.record();
        vm.startPrank(PRANK_ADDR_2);
        victim.record();
        storedSender = victim.lastSender();
    }

    // -----------------------------------------------------------------
    // Nested calls
    // -----------------------------------------------------------------

    function callPrankNested() external {
        vm.prank(NESTED_ADDR);
        victim.nestedRecord(inner);
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    function callStartNested() external {
        vm.startPrank(START_ADDR, START_ORIGIN);
        victim.nestedRecord(inner);
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    // -----------------------------------------------------------------
    // Constructor pranking
    // -----------------------------------------------------------------

    function callPrankConstructor() external {
        vm.prank(CONSTRUCTOR_ADDR);
        PrankVictim v = new PrankVictim();
        storedSender = v.lastSender();
        v.record();
    }

    // -----------------------------------------------------------------
    // Modifier with startPrank / stopPrank
    // -----------------------------------------------------------------

    function callModifierPrank(uint256 actorSeed) external useActor(actorSeed) {
        victim.record();
    }

    // -----------------------------------------------------------------
    // Single-transaction sequence determinism
    // -----------------------------------------------------------------

    function callPrankSequence() external {
        vm.prank(PRANK_ADDR);
        victim.record();
        seqSender1 = victim.lastSender();

        vm.prank(PRANK_ADDR_2, PRANK_ORIGIN);
        victim.record();
        seqSender2 = victim.lastSender();
        seqOrigin2 = victim.lastOrigin();

        vm.startPrank(START_ADDR);
        victim.record();
        seqSender3 = victim.lastSender();

        vm.stopPrank();
        victim.record();
        seqSender4 = victim.lastSender();
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    function callPrankAndDeal() external {
        vm.prank(PRANK_ADDR);
        vm.deal(address(this), 5 ether);
        storedBalance = address(this).balance;
        victim.record();
        storedSender = victim.lastSender();
    }

    function callStartPrankAndWarp() external {
        vm.startPrank(START_ADDR);
        vm.warp(1234567890);
        storedTimestamp = block.timestamp;
        victim.record();
        storedSender = victim.lastSender();
        vm.stopPrank();
    }

    // -----------------------------------------------------------------
    // Fuzzing actions
    // -----------------------------------------------------------------

    function actionPrank() external {
        vm.prank(PRANK_ADDR);
        victim.record();
        storedSender = victim.lastSender();
    }

    function actionStartPrank() external {
        vm.startPrank(PERSIST_ADDR);
        victim.record();
        storedSender = victim.lastSender();
    }

    function actionStopPrank() external {
        vm.stopPrank();
        victim.record();
        storedSender = victim.lastSender();
    }

    function actionRestore() external {
        victim.record();
        storedSender = victim.lastSender();
        storedOrigin = victim.lastOrigin();
    }

    function actionModifierPrank(
        uint256 actorSeed
    ) external useActor(actorSeed) {
        victim.record();
        storedSender = victim.lastSender();
    }

    // -----------------------------------------------------------------
    // Invariants
    // -----------------------------------------------------------------

    function invariant_prank() external view {
        assert(
            storedSender == address(this) ||
                storedSender == PRANK_ADDR ||
                storedSender == PRANK_ADDR_2 ||
                storedSender == START_ADDR ||
                storedSender == PERSIST_ADDR ||
                storedSender == NESTED_ADDR ||
                storedSender == CONSTRUCTOR_ADDR ||
                storedSender == actors[0] ||
                storedSender == actors[1] ||
                storedSender == actors[2]
        );
    }

    function invariant_victim_sender() external view {
        assert(
            victim.lastSender() == address(this) ||
                victim.lastSender() == PRANK_ADDR ||
                victim.lastSender() == PRANK_ADDR_2 ||
                victim.lastSender() == START_ADDR ||
                victim.lastSender() == PERSIST_ADDR ||
                victim.lastSender() == NESTED_ADDR ||
                victim.lastSender() == CONSTRUCTOR_ADDR ||
                victim.lastSender() == actors[0] ||
                victim.lastSender() == actors[1] ||
                victim.lastSender() == actors[2]
        );
    }

    function invariant_modifier_prank() external view {
        assert(
            victim.lastSender() == address(this) ||
                victim.lastSender() == actors[0] ||
                victim.lastSender() == actors[1] ||
                victim.lastSender() == actors[2]
        );
    }
}
