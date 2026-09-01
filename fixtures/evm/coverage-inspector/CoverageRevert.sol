// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract CoverageRevert {
    function maybeRevert(bool shouldRevert) external {
        if (shouldRevert) {
            revert();
        }
    }
}
