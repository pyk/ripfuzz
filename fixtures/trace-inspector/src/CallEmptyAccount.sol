// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IEmptyTarget {
    function value() external view returns (uint256);
}

/// @notice Calls a derived address with no bytecode so the trace can show
/// the empty-account path (EVM STOP + parent empty revert).
contract CallEmptyAccount {
    // Same derivation style as the RVM address: keccak256 of a label string.
    address constant TARGET = address(uint160(uint256(keccak256("empty account target"))));

    constructor() {
        IEmptyTarget(TARGET).value();
    }

    function set(uint256 x) external {
        // unreachable; present so the contract loads as a harness artifact
    }
}
