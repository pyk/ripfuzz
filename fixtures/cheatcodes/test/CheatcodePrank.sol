// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";
import {PrankVictim} from "../src/PrankVictim.sol";

contract CheatcodePrank {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));
    PrankVictim public victim;
    PrankVictim public inner;
    address[] public actors;
    address public currentActor;

    function setup() external {
        victim = new PrankVictim();
        inner = new PrankVictim();
        actors = [address(0x1234), address(0x5678), address(0x9abc)];
    }

    modifier useActor(uint256 actorSeed) {
        currentActor = actors[actorSeed % actors.length];
        vm.startPrank(currentActor);
        _;
        vm.stopPrank();
    }

    // 1. vm.prank(addr) changes msg.sender only, not tx.origin
    function call_prank_sender() external {
        address oldOrigin = tx.origin;
        vm.prank(address(0x111));
        victim.record();
        require(victim.lastSender() == address(0x111), "prank sender wrong");
        require(victim.lastOrigin() == oldOrigin, "prank origin mutated");
    }

    // 2. vm.prank(addr, origin) changes both
    function call_prank_origin() external {
        vm.prank(address(0x222), address(0x333));
        victim.record();
        require(victim.lastSender() == address(0x222), "prank sender wrong");
        require(victim.lastOrigin() == address(0x333), "prank origin wrong");
    }

    // 3. vm.prank is consumed by the next call and cleaned up
    function call_prank_consumed() external {
        address oldOrigin = tx.origin;
        vm.prank(address(0x444));
        victim.record();
        // second call must NOT be pranked
        victim.record();
        require(victim.lastSender() == address(this), "prank leaked");
        require(victim.lastOrigin() == oldOrigin, "origin leaked");
    }

    // 4. vm.startPrank / stopPrank
    function call_start_stop() external {
        address oldOrigin = tx.origin;
        vm.startPrank(address(0x555), address(0x666));
        victim.record();
        victim.record();
        require(victim.lastSender() == address(0x555), "startPrank sender");
        require(victim.lastOrigin() == address(0x666), "startPrank origin");
        vm.stopPrank();
        victim.record();
        // after stopPrank
        require(victim.lastSender() == address(this), "stopPrank sender");
        require(victim.lastOrigin() == oldOrigin, "stopPrank origin");
    }

    function debug_last_sender() external view returns (address) {
        return victim.lastSender();
    }

    // 5. vm.startPrank without stopPrank persists across calls
    function call_start_no_stop() external {
        vm.startPrank(address(0x777));
        victim.record();
    }

    function call_after_start_no_stop() external {
        victim.record();
        require(victim.lastSender() == address(0x777), "startPrank persisted");
    }

    function call_after_stop() external {
        victim.record();
        require(victim.lastSender() == address(this), "stopPrank failed");
    }

    // 6. Overwrite validation: startPrank can overwrite a used startPrank
    function call_start_overwrite_used() external {
        vm.startPrank(address(0x111));
        victim.record();
        vm.startPrank(address(0x222));
        victim.record();
    }

    // 7. Overwrite validation: unused startPrank cannot be overwritten
    function call_start_overwrite_unused_reverts() external {
        vm.startPrank(address(0x111));
        vm.startPrank(address(0x222));
        victim.record();
    }

    // 8. Overwrite validation: prank cannot overwrite startPrank
    function call_prank_over_start_reverts() external {
        vm.startPrank(address(0x111));
        vm.prank(address(0x222));
        victim.record();
    }

    // 9. Overwrite validation: double prank reverts
    function call_double_prank_reverts() external {
        vm.prank(address(0x111));
        vm.prank(address(0x222));
        victim.record();
    }

    // 10. Nested calls: only the immediate next call is pranked, not deeper calls from the victim
    function call_prank_nested() external {
        address oldOrigin = tx.origin;
        vm.prank(address(0x999));
        victim.nestedRecord(inner);
        require(victim.lastSender() == address(0x999), "outer pranked");
        require(victim.lastOrigin() == oldOrigin, "outer origin");
        // inner was called BY victim, so it should see victim as sender
        require(inner.lastSender() == address(victim), "inner leaked");
    }

    // 8. startPrank nested calls: all calls are pranked
    function call_start_nested() external {
        vm.startPrank(address(0xaaa), address(0xbbb));
        victim.nestedRecord(inner);
        require(victim.lastSender() == address(0xaaa), "outer startPrank");
        require(victim.lastOrigin() == address(0xbbb), "outer origin");
        require(inner.lastSender() == address(0xaaa), "inner startPrank");
        require(inner.lastOrigin() == address(0xbbb), "inner origin");
        vm.stopPrank();
    }

    // 9. stopPrank mid-sequence
    function call_stop_mid() external {
        vm.stopPrank();
        victim.record();
        require(victim.lastSender() == address(this), "stopPrank failed");
    }

    // 10. Constructor pranking
    function call_prank_constructor() external {
        vm.prank(address(0xccc));
        PrankVictim v = new PrankVictim();
        require(v.lastSender() == address(0xccc), "constructor prank");
        // after creation, normal caller restored
        v.record();
        require(v.lastSender() == address(this), "post-constructor");
    }

    // 11. Modifier with startPrank / stopPrank
    function call_modifier_prank() external useActor(0) {
        victim.record();
    }

    // --- Checks ---

    function prank_sender_ok() external view returns (bool) {
        return victim.lastSender() == address(0x111);
    }

    function prank_origin_ok() external view returns (bool) {
        return victim.lastSender() == address(0x222)
            && victim.lastOrigin() == address(0x333);
    }

    function start_persisted() external view returns (bool) {
        return victim.lastSender() == address(0x777);
    }

    function start_overwrite_ok() external view returns (bool) {
        return victim.lastSender() == address(0x222);
    }

    function nested_ok() external view returns (bool) {
        return victim.lastSender() == address(0x999)
            && inner.lastSender() == address(victim);
    }

    function start_nested_ok() external view returns (bool) {
        return victim.lastSender() == address(0xaaa)
            && inner.lastSender() == address(0xaaa);
    }

    function modifier_prank_ok() external view returns (bool) {
        return victim.lastSender() == actors[0];
    }
}
