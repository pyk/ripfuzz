// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract DealTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant DEAL_TARGET = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    uint256 constant EXPECTED_BALANCE = 1000 ether;

    uint256 public storedBalance;

    function setup() external {
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
        storedBalance = DEAL_TARGET.balance;
    }

    function getBalance(address addr) external view returns (uint256) {
        return addr.balance;
    }

    /// Call vm.deal with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callDealSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.deal(address(this), EXPECTED_BALANCE);
        first = address(this).balance;
        vm.deal(address(this), EXPECTED_BALANCE);
        second = address(this).balance;
    }

    /// Call vm.deal with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callDealSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.deal(address(this), 1 ether);
        first = address(this).balance;
        vm.deal(address(this), EXPECTED_BALANCE);
        second = address(this).balance;
        vm.deal(address(this), 5 ether);
        third = address(this).balance;
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callDealAndWarp()
        external
        returns (uint256 balance, uint256 timestamp)
    {
        vm.deal(address(this), EXPECTED_BALANCE);
        vm.warp(1234567890);
        balance = address(this).balance;
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-deal the expected balance and store it.
    function actionDeal() external {
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
        storedBalance = DEAL_TARGET.balance;
    }

    function invariant_deal() external view {
        assert(storedBalance == EXPECTED_BALANCE);
    }
}
