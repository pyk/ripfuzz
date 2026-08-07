// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract GetEnvHarness {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    /// Always-present process environment key used for defined-value tests.
    string constant DEFINED_KEY = "PATH";
    string constant EXPECTED_DEFAULT = "default-value";
    string constant MISSING_KEY = "RIPFUZZ_TEST_GET_ENV_MISSING_XYZ";

    string public storedValue;

    function setup() external {
        // Seed via the default overload so setup does not depend on a custom env.
        storedValue = vm.getEnv(MISSING_KEY, EXPECTED_DEFAULT);
    }

    /// Restore the canonical stored value via the default overload.
    function actionGetEnvOrDefault() external {
        storedValue = vm.getEnv(MISSING_KEY, EXPECTED_DEFAULT);
    }

    /// Overwrite stored value via the default overload using a different default.
    function actionMutateViaDefault() external {
        storedValue = vm.getEnv(MISSING_KEY, "mutated");
    }

    /// Read a missing environment variable without a default (expected to revert).
    function actionGetEnvMissing() external {
        storedValue = vm.getEnv(MISSING_KEY);
    }

    function getStoredValue() external view returns (string memory) {
        return storedValue;
    }

    /// Read a defined environment variable directly from the cheatcode inspector.
    function getEnvDirect(string calldata key) external returns (string memory) {
        return vm.getEnv(key);
    }

    /// Read an environment variable with a default, directly.
    function getEnvOrDefaultDirect(string calldata key, string calldata defaultValue) external returns (string memory) {
        return vm.getEnv(key, defaultValue);
    }

    /// Invariant: stored value must match the canonical default-seeded value.
    function invariant_getEnv() external view {
        assert(keccak256(bytes(storedValue)) == keccak256(bytes(EXPECTED_DEFAULT)));
    }
}
