// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "./RVM.sol";

struct Point {
    uint256 x;
    uint256 y;
}

struct Line {
    Point start;
    Point end;
}

library MathLib {
    function add(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}

contract Counter {
    uint256 public value;
    string public strValue;
    bool public boolValue;
    uint256[] public arrValues;
    mapping(uint256 => uint256) public mapValues;
    bytes public byteValue;
    bytes32 public fixedByteValue;

    constructor(uint256 initValue) {
        value = initValue;
    }

    function increment() external {
        value += 1;
    }

    function setValue(uint256 x) external {
        value = x;
    }

    function getValue() external view returns (uint256) {
        return value;
    }

    function setString(string calldata s) external {
        strValue = s;
    }

    function setBool(bool b) external {
        boolValue = b;
    }

    function pushArray(uint256 x) external {
        arrValues.push(x);
    }

    function setMap(uint256 k, uint256 v) external {
        mapValues[k] = v;
    }

    function setArray(uint256[] calldata arr) external {
        for (uint256 i = 0; i < arr.length; i++) {
            arrValues.push(arr[i]);
        }
    }

    function setStruct(Point calldata p) external {
        value = p.x + p.y;
    }

    function setNestedStruct(Line calldata l) external {
        value = l.start.x + l.start.y + l.end.x + l.end.y;
    }

    function setBytes(bytes calldata b) external {
        byteValue = b;
    }

    function setFixedBytes(bytes32 b) external {
        fixedByteValue = b;
    }

    function doInternal() external {
        _internalHelper(5);
    }

    function _internalHelper(uint256 x) internal {
        value += x;
    }

    function doLibraryCall(uint256 a, uint256 b) external pure returns (uint256) {
        return MathLib.add(a, b);
    }

    function selfCall(uint256 x) external {
        this.setValue(x);
    }

    function payableCall() external payable {
        value = msg.value;
    }
}

contract CounterHelper {
    function help(Counter counter) external {
        counter.setValue(999);
        counter.setBool(false);
    }
}

contract DeepContract {
    CounterHelper public helper;

    constructor() {
        helper = new CounterHelper();
    }

    function run(Counter counter) external {
        counter.increment();
        helper.help(counter);
    }
}

contract BasicConstructorComplexRevert {
    RVM public constant rvm = RVM(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    constructor() payable {
        Counter counter = new Counter(42);
        counter.increment();
        counter.increment();
        counter.setValue(42);
        counter.getValue();
        counter.setValue(100);
        counter.setString("hello");
        counter.setBool(true);
        counter.pushArray(1);
        counter.pushArray(2);
        counter.setMap(10, 100);
        uint256[] memory arr = new uint256[](3);
        arr[0] = 3;
        arr[1] = 4;
        arr[2] = 5;
        counter.setArray(arr);
        counter.setStruct(Point({ x: 7, y: 8 }));
        counter.setNestedStruct(Line({ start: Point({ x: 1, y: 2 }), end: Point({ x: 3, y: 4 }) }));
        counter.setBytes(hex"deadbeef");
        counter.setFixedBytes(hex"cafebabe00000000000000000000000000000000000000000000000000000000");
        counter.doInternal();
        counter.doLibraryCall(10, 20);
        MathLib.add(10, 20);
        counter.selfCall(50);
        counter.payableCall{value: 1000}();
        rvm.warp(1234567890);
        DeepContract deep = new DeepContract();
        deep.run(counter);
        revert("constructor always reverts");
    }

    function set(uint256 x) external {
        // unreachable
    }
}
