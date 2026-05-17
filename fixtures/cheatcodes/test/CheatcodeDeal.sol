// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeDeal {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    uint256 public recordedBalance;
    address public constant TARGET = address(0xBEEF);
    address public constant EMPTY_ADDR = address(0xDEAD);

    // --- setUp interaction ---

    function setUp() external {
        vm.deal(address(this), 5 ether);
        vm.deal(TARGET, 3 ether);
    }

    function call_record_target_balance() external {
        recordedBalance = TARGET.balance;
    }

    function property_setup_deal_persists() external view returns (bool) {
        return address(this).balance == 5 ether && TARGET.balance == 3 ether;
    }

    function property_setup_only() external view returns (bool) {
        return address(this).balance == 5 ether;
    }

    // --- Same-sequence persistence ---

    function call_deal(uint256 amt) external {
        vm.deal(TARGET, amt);
        recordedBalance = TARGET.balance;
    }

    function property_deal_persists_across_calls() external view returns (bool) {
        // call_deal(100) -> next call sees 100
        return recordedBalance == 100;
    }

    // --- Revert safety ---

    function call_deal_and_revert(uint256 amt) external {
        vm.deal(TARGET, amt);
        revert("intentional");
    }

    function property_revert_undoes_deal() external view returns (bool) {
        // setUp dealt 3 ether; if call_deal_and_revert reverted, balance must be 3 ether
        return TARGET.balance == 3 ether;
    }

    // --- Overwrite ---

    function call_deal_100() external {
        vm.deal(TARGET, 100);
    }

    function call_deal_200() external {
        vm.deal(TARGET, 200);
    }

    function property_deal_overwrite() external view returns (bool) {
        return TARGET.balance == 200;
    }

    // --- Edge: zero ---

    function call_deal_zero() external {
        vm.deal(TARGET, 0);
    }

    function property_deal_zero() external view returns (bool) {
        return TARGET.balance == 0;
    }

    // --- Edge: max uint256 ---

    function call_deal_max() external {
        vm.deal(TARGET, type(uint256).max);
    }

    function property_deal_max() external view returns (bool) {
        return TARGET.balance == type(uint256).max;
    }

    // --- Edge: empty / non-existent address ---

    function call_deal_empty(uint256 amt) external {
        vm.deal(EMPTY_ADDR, amt);
    }

    function property_deal_empty() external view returns (bool) {
        return EMPTY_ADDR.balance == 42;
    }

    // --- Property sees final deal ---

    function property_final_balance() external view returns (bool) {
        // If the only call was call_deal_100(), property should see 100
        return TARGET.balance == 100;
    }

    // --- Cross-cheatcode interaction: deal + warp + roll ---

    function call_deal_and_warp_roll() external {
        vm.deal(TARGET, 777);
        vm.warp(12345);
        vm.roll(67890);
    }

    function property_deal_and_warp_roll() external view returns (bool) {
        return TARGET.balance == 777
            && block.timestamp == 12345
            && block.number == 67890;
    }

    // --- Self-deal (contract deals to itself mid-sequence) ---

    function call_self_deal(uint256 amt) external {
        vm.deal(address(this), amt);
    }

    function property_self_deal_overwrites_setup() external view returns (bool) {
        // setUp dealt 5 ether, then call_self_deal(1 ether) should overwrite
        return address(this).balance == 1 ether;
    }
}
