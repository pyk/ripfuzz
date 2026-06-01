// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract LogEvents {
    event Log(string message);
    event Log(string message, string value);
    event Log(string message, uint256 value);
    event Log(string message, address value);
    event Log(string message, bytes value);
    event Log(string message, bool value);

    function dummy() external {}

    constructor() {
        string memory m1 = "simple log";
        emit Log(m1);

        string memory m2 = "string value: ";
        string memory v2 = "hello";
        emit Log(m2, v2);

        string memory m3 = "uint256 value: ";
        uint256 v3 = 42;
        emit Log(m3, v3);

        string memory m4 = "address value: ";
        address v4 = address(0xCAFE);
        emit Log(m4, v4);

        string memory m5 = "bytes value: ";
        bytes memory v5 = hex"1234";
        emit Log(m5, v5);

        string memory m6 = "bool value: ";
        bool v6 = true;
        emit Log(m6, v6);
    }
}
