// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithAssert {
    function always_panics() external pure {
        assert(false);
    }
}
