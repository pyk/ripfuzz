// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness with two independent bugs. Each handler always asserts, so the
/// fuzzer can discover `bugA()` and `bugB()` as distinct assertions.
contract MultiFail {
    function bugA() external {
        assert(1 == 2);
    }

    function bugB() external {
        assert(2 == 3);
    }
}
