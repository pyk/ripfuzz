// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

struct Point {
    uint256 x;
    uint256 y;
}

struct Line {
    Point start;
    Point end;
}

contract ReturnValueTypes {
    function returnBool() external pure returns (bool) {
        return true;
    }

    function returnUint256() external pure returns (uint256) {
        return 42;
    }

    function returnInt256() external pure returns (int256) {
        return -42;
    }

    function returnAddress() external pure returns (address) {
        return 0xD93a248535Ef447440e7D63A2aff6c3e75B235C7;
    }

    function returnBytes32() external pure returns (bytes32) {
        return hex"deadbeef00000000000000000000000000000000000000000000000000000000";
    }

    function returnString() external pure returns (string memory) {
        return "hello";
    }

    function returnBytes() external pure returns (bytes memory) {
        return hex"cafebabe";
    }

    function returnStruct() external pure returns (Point memory) {
        return Point(1, 2);
    }

    function returnArray() external pure returns (uint256[] memory) {
        uint256[] memory arr = new uint256[](3);
        arr[0] = 1;
        arr[1] = 2;
        arr[2] = 3;
        return arr;
    }

    function returnFixedArray() external pure returns (uint256[3] memory) {
        return [uint256(10), 20, 30];
    }

    function returnMultiple() external pure returns (uint256, string memory) {
        return (100, "world");
    }

    function returnNestedStruct() external pure returns (Line memory) {
        return Line(Point(1, 2), Point(3, 4));
    }
}
