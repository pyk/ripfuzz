// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";

/// @notice Harness that pranks a caller around a value transfer.
contract PrankTransferHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address constant SPONSOR = address(0xBEEF);

    /// Prank a zero-balance caller around a 1 wei transfer to the fuzz
    /// caller. The transfer must revert out-of-funds instead of panicking
    /// the EVM journal.
    function actionPrankUnfundedTransfer() external {
        rvm.prank(SPONSOR);
        payable(msg.sender).transfer(1);
    }

    /// Fund the pranked caller, then transfer 1 wei to this contract.
    /// The pranked caller pays the value, so the transfer succeeds.
    function actionPrankFundedTransfer() external {
        rvm.deal(SPONSOR, 1);
        rvm.prank(SPONSOR);
        payable(address(this)).transfer(1);
    }

    receive() external payable {}
}
