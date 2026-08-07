// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

contract GetEnvHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Always-present process environment key used for defined-value tests.
    string constant DEFINED_KEY = "PATH";
    string constant EXPECTED_DEFAULT = "default-value";
    string constant MISSING_KEY = "RIPFUZZ_TEST_GET_ENV_MISSING_XYZ";

    string public storedValue;

    function setup() external {
        // Seed via the default overload so setup does not depend on a custom env.
        storedValue = rvm.getEnv(MISSING_KEY, EXPECTED_DEFAULT);
    }

    /// Restore the canonical stored value via the default overload.
    function actionGetEnvOrDefault() external {
        storedValue = rvm.getEnv(MISSING_KEY, EXPECTED_DEFAULT);
    }

    /// Overwrite stored value via the default overload using a different default.
    function actionMutateViaDefault() external {
        storedValue = rvm.getEnv(MISSING_KEY, "mutated");
    }

    /// Read a missing environment variable without a default (expected to revert).
    function actionGetEnvMissing() external {
        storedValue = rvm.getEnv(MISSING_KEY);
    }

    function getStoredValue() external view returns (string memory) {
        return storedValue;
    }

    /// Read a defined environment variable directly from the cheatcode inspector.
    function getEnvDirect(string calldata key) external returns (string memory) {
        return rvm.getEnv(key);
    }

    /// Read an environment variable with a default, directly.
    function getEnvOrDefaultDirect(string calldata key, string calldata defaultValue) external returns (string memory) {
        return rvm.getEnv(key, defaultValue);
    }

    /// Invariant: stored value must match the canonical default-seeded value.
    function invariant_getEnv() external view {
        assert(keccak256(bytes(storedValue)) == keccak256(bytes(EXPECTED_DEFAULT)));
    }
}
