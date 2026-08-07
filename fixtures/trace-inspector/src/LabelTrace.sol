// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "./RVM.sol";

contract LabelTrace {
    RVM public constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    event LabeledTransfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        rvm.label(address(this), "SelfLabel");
        rvm.label(address(0xBEEF), "BeefLabel");

        rvm.deal(address(0xBEEF), 100);
        emit LabeledTransfer(address(0xBEEF), address(this), 1000);

        revert("constructor always reverts");
    }
}
