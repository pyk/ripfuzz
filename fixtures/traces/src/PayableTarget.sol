// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract PayableTarget {
    uint256 public balanceReceived;

    receive() external payable {}

    function deposit() external payable {
        balanceReceived += msg.value;
    }
}
