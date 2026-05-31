// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract EmitEvents {
    struct Point {
        uint256 x;
        uint256 y;
    }

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event NoArgs();
    event IndexedBytes(address indexed sender, bytes data);
    event AnonymousEvent(uint256 value) anonymous;
    event MultipleIndexed(uint256 indexed a, address indexed b, bytes32 indexed c, uint256 d);
    event StringEvent(string name);
    event StructEvent(Point p);
    event ArrayEvent(uint256[] values);

    function dummy() external {}

    constructor() {
        emit Transfer(address(0), msg.sender, 1000);
        emit Approval(msg.sender, address(0xBEEF), 500);
        emit NoArgs();
        emit IndexedBytes(msg.sender, hex"1234");
        emit AnonymousEvent(42);
        emit MultipleIndexed(1, address(0xCAFE), bytes32(uint256(2)), 3);
        emit StringEvent("hello");
        emit StructEvent(Point(1, 2));
        uint256[] memory arr = new uint256[](3);
        arr[0] = 1;
        arr[1] = 2;
        arr[2] = 3;
        emit ArrayEvent(arr);
    }
}
