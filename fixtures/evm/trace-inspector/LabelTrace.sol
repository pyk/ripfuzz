// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {RVM} from "./RVM.sol";

contract LabelTrace {
    RVM public constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    event LabeledTransfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        rvm.label(address(this), "SelfLabel");
        rvm.label(address(0xBEEF), "BeefLabel");

        rvm.deal(address(0xBEEF), 100);
        emit LabeledTransfer(address(0xBEEF), address(this), 1000);

        revert("constructor always reverts");
    }
}
