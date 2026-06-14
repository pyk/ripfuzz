// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract PrankHandler {
    address public lastSender;

    function record() external {
        lastSender = msg.sender;
    }
}
