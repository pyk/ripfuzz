// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// A contract whose invariant can never be triggered.
/// The fuzzer should run the full number of iterations without finding a failure.
contract ImpossibleBug {
    uint256 public counter;
    mapping(uint256 => uint256) public values;

    function increment() external {
        counter += 1;
    }

    function decrement() external {
        require(counter > 0, "underflow");
        counter -= 1;
    }

    function add(uint256 x) external {
        counter += x;
    }

    function store(uint256 key, uint256 val) external {
        values[key] = val;
    }

    function clear(uint256 key) external {
        delete values[key];
    }

    function invariant_never_triggered() external pure {
        assert(true); // intentionally never crashes
    }
}
