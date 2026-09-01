// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

interface RVM {
    struct Invariant {
        string id;
        string description;
    }

    function bail(Invariant calldata invariant) external;
}

contract HarnessWithFailingHandler {
    uint256 public total;

    address constant RVM_ADDRESS = address(uint160(uint256(keccak256("ripfuzz cheatcode"))));

    function deposit(uint256 amount) external {
        total += amount;
        if (total >= 1000) {
            RVM(RVM_ADDRESS).bail(RVM.Invariant({id: "HAN-001", description: "total exceeded 1000"}));
        }
    }

    function invariant_total() external {
        if (total >= type(uint256).max) {
            RVM(RVM_ADDRESS).bail(RVM.Invariant({id: "INV-MAX", description: "total overflowed"}));
        }
    }
}
