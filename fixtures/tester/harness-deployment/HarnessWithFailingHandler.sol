// SPDX-License-Identifier: MIT
pragma solidity ^0.8.36;

interface RVM {
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
    function finding(Finding calldata finding) external;
}

contract HarnessWithFailingHandler {
    uint256 public total;

    address constant RVM_ADDRESS = address(uint160(uint256(keccak256("ripfuzz cheatcode"))));

    function deposit(uint256 amount) external {
        total += amount;
        if (total >= 1000) {
            RVM(RVM_ADDRESS)
                .finding(
                    RVM.Finding({
                    id: "HAN-001",
                    severity: RVM.Severity.Critical,
                    title: "total below 1000",
                    description: "total exceeded 1000"
                })
                );
        }
    }

    function invariant_total() external {
        if (total >= type(uint256).max) {
            RVM(RVM_ADDRESS)
                .finding(
                    RVM.Finding({id: "INV-MAX", severity: RVM.Severity.High, title: "total below max", description: ""})
                );
        }
    }
}
