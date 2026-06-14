// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title DealHandler
/// @notice Real-world fuzz handler that funds accounts via the `vm.deal`
///         cheatcode. Setup establishes a canonical balance and actions mutate
///         or restore it. Invariants verify deterministic control.
contract DealHandler {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address constant DEAL_TARGET = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    uint256 constant EXPECTED_BALANCE = 1000 ether;

    function setup() external {
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
    }

    /// Invariant: the live balance of the target account must always match
    /// the expected value.
    function invariant_deal() external view {
        assert(DEAL_TARGET.balance == EXPECTED_BALANCE);
    }

    /// Action: re-deal the expected balance to the target account.
    function actionRestoreDeal() external {
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
    }

    /// Action: temporarily deal a different balance to the target account.
    function actionMutateDeal() external {
        vm.deal(DEAL_TARGET, 1 ether);
    }

    /// Action: interleave deal changes inside one tx, ending on expected.
    function actionDealSequence() external {
        vm.deal(DEAL_TARGET, 5 ether);
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
        vm.deal(DEAL_TARGET, 1 ether);
        vm.deal(DEAL_TARGET, EXPECTED_BALANCE);
    }
}
