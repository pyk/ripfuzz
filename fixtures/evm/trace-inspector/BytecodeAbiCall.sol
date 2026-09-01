// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Callee with no ABI registered in the trace context. Argument types must
/// come from bytecode (evmole), not the project artifact.
contract BytecodeAbiTarget {
    address public lastFrom;
    uint256 public lastAmount;
    address public lastTo;

    function drain(address from, uint256 amount, address to) external {
        lastFrom = from;
        lastAmount = amount;
        lastTo = to;
    }
}

/// Deploys [`BytecodeAbiTarget`] and calls `drain` so the inner CALL is
/// present in the constructor trace.
contract BytecodeAbiCall {
    constructor() {
        BytecodeAbiTarget t = new BytecodeAbiTarget();
        t.drain(address(0x1111), 8453, address(0x2222));
        revert("bytecode abi call");
    }

    function set(uint256 x) external {
        // unreachable; present so the contract loads as a harness artifact
    }
}
