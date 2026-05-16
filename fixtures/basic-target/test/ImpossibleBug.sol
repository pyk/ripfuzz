// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// A contract whose property can never be triggered.
/// The fuzzer should run the full number of iterations without finding a failure.
contract ImpossibleBug {
    function property_never_triggered() external view returns (bool) {
        return false;
    }
}
