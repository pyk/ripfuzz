// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract HarnessWithPayableInvariant {
    function deposit(uint256 amount) external pure {
        require(amount > 0, "empty");
    }

    function invariant_total() external payable {
        assert(true);
    }
}
