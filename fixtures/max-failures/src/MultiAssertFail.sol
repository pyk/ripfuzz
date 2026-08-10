// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness where one handler function contains two independent assertions.
/// Both branches fail with `assert`, so the fuzzer must treat the two
/// assertion PCs as two distinct bugs.
contract MultiAssertFail {
    function fail(bool first) external {
        if (first) {
            assert(1 == 2);
        } else {
            assert(2 == 3);
        }
    }
}
