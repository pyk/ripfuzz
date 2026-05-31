// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "./Vm.sol";

contract LabelTrace {
    Vm public constant VM = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    event LabeledTransfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        VM.label(address(this), "SelfLabel");
        VM.label(address(0xBEEF), "BeefLabel");

        VM.deal(address(0xBEEF), 100);
        emit LabeledTransfer(address(0xBEEF), address(this), 1000);

        revert("constructor always reverts");
    }
}
