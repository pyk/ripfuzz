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
            RVM(RVM_ADDRESS)
                .finding(
                    RVM.Finding({
                    id: "INV-001",
                    severity: RVM.Severity.High,
                    title: "total below limit",
                    description: "total exceeded 100"
                })
                );
        }
    }
}
