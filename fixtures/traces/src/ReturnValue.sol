// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ReturnValue {
    function getBool() external pure returns (bool) {
        return true;
    }

    function getString() external pure returns (string memory) {
        return "hello world";
    }

    function getAddress() external view returns (address) {
        return address(this);
    }

    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
