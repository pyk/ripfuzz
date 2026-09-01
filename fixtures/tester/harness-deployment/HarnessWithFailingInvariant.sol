// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

interface RVM {
    struct Invariant {
        string id;
        string description;
    }

    function bail(Invariant calldata invariant) external;
}

contract HarnessWithFailingInvariant {
    uint256 public total;

    address constant RVM_ADDRESS = address(uint160(uint256(keccak256("ripfuzz cheatcode"))));

    function increment(uint256 amount) external {
        total += amount;
    }

    function reset(uint256 value) external {
        total = value;
    }

    function invariant_total_below_limit() external {
        if (total > 100) {
            RVM(RVM_ADDRESS).bail(RVM.Invariant({id: "INV-001", description: "total exceeded 100"}));
        }
    }
}
