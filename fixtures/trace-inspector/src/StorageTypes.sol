// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

struct Point {
    uint256 x;
    uint256 y;
}

struct Line {
    Point start;
    Point end;
}

contract StorageTypes {
    bool public boolValue;
    uint8 public uint8Value;
    uint128 public uint128Value;
    uint256 public uint256Value;
    int8 public int8Value;
    int128 public int128Value;
    int256 public int256Value;
    bytes1 public bytes1Value;
    bytes32 public bytes32Value;
    address public addressValue;
    bytes public bytesValue;
    string public stringValue;
    uint256[] public uintArray;
    Point[] public structArray;
    uint256[3] public fixedArray;

    constructor() {
        boolValue = true;
        uint8Value = 255;
        uint128Value = 340282366920938463463374607431768211455;
        uint256Value = 115792089237316195423570985008687907853269984665640564039457584007913129639935;
        int8Value = -128;
        int128Value = -170141183460469231731687303715884105728;
        int256Value = -57896044618658097711785492504343953926634992332820282019728792003956564819968;
        bytes1Value = 0x01;
        bytes32Value = hex"deadbeef00000000000000000000000000000000000000000000000000000000";
        addressValue = 0xC34296175b9e78F66EDbeaEb7acEa4c615C092E1;
        bytesValue = hex"cafebabe";
        stringValue = "hello";
        uintArray.push(1);
        uintArray.push(2);
        structArray.push(Point({x: 1, y: 2}));
        structArray.push(Point({x: 3, y: 4}));
        fixedArray[0] = 10;
        fixedArray[1] = 20;
        fixedArray[2] = 30;
    }

    function setBool(bool b) external {
        boolValue = b;
    }

    function setUint8(uint8 x) external {
        uint8Value = x;
    }

    function setUint128(uint128 x) external {
        uint128Value = x;
    }

    function setUint256(uint256 x) external {
        uint256Value = x;
    }

    function setInt8(int8 x) external {
        int8Value = x;
    }

    function setInt128(int128 x) external {
        int128Value = x;
    }

    function setInt256(int256 x) external {
        int256Value = x;
    }

    function setBytes1(bytes1 b) external {
        bytes1Value = b;
    }

    function setBytes32(bytes32 b) external {
        bytes32Value = b;
    }

    function setAddress(address a) external {
        addressValue = a;
    }

    function setBytes(bytes calldata b) external {
        bytesValue = b;
    }

    function setString(string calldata s) external {
        stringValue = s;
    }

    function pushUintArray(uint256 x) external {
        uintArray.push(x);
    }

    function pushStructArray(Point calldata p) external {
        structArray.push(p);
    }

    function setFixedArray(uint256 i, uint256 v) external {
        fixedArray[i] = v;
    }

    function setTuple(uint256 a, uint256 b) external {
        (uint256Value, uint128Value) = (a, uint128(b));
    }

    function setStruct(Point calldata p) external {
        uint256Value = p.x + p.y;
    }

    function setNestedStruct(Line calldata l) external {
        uint256Value = l.start.x + l.start.y + l.end.x + l.end.y;
    }
}

contract StorageTypesRevert {
    constructor() {
        StorageTypes st = new StorageTypes();
        st.setBool(false);
        st.setUint8(0);
        st.setUint128(0);
        st.setUint256(0);
        st.setInt8(0);
        st.setInt128(0);
        st.setInt256(0);
        st.setBytes1(0);
        st.setBytes32(hex"0000000000000000000000000000000000000000000000000000000000000000");
        st.setAddress(address(0));
        st.setBytes(hex"");
        st.setString("");
        st.pushUintArray(3);
        st.pushStructArray(Point({x: 5, y: 6}));
        st.setFixedArray(0, 0);
        st.setFixedArray(1, 0);
        st.setFixedArray(2, 0);
        st.setTuple(0, 0);
        st.setStruct(Point({x: 0, y: 0}));
        st.setNestedStruct(Line({start: Point({x: 0, y: 0}), end: Point({x: 0, y: 0})}));
        revert("storage types revert");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
