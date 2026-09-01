// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

struct Invariant {
    string id;
    string description;
}

interface RVM {
    function bail(Invariant calldata invariant) external;
}

abstract contract Challenge {
    RVM constant rvm = RVM(address(uint160(uint256(keccak256("ripfuzz cheatcode")))));
}
