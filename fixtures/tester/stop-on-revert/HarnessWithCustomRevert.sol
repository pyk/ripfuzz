// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

error CustomRevert();

contract HarnessWithCustomRevert {
    function always_reverts() external pure {
        revert CustomRevert();
    }
}
