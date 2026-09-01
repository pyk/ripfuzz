// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical
}

struct Finding {
    string id;
    Severity severity;
    string title;
    string description;
}

interface RVM {
    function finding(Finding calldata finding) external;
    function finding(string calldata id) external;
}

abstract contract Challenge {
    RVM constant rvm = RVM(address(uint160(uint256(keccak256("ripfuzz cheatcode")))));
}
