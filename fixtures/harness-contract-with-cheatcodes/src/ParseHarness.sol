// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

contract ParseHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    uint256 constant EXPECTED_UINT = 123;
    int256 constant EXPECTED_INT = -42;
    bool constant EXPECTED_BOOL = true;
    address constant EXPECTED_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    bytes32 constant EXPECTED_BYTES32 = 0x7465737400000000000000000000000000000000000000000000000000000000;

    uint256 public storedUint;
    int256 public storedInt;
    bool public storedBool;
    address public storedAddr;
    bytes32 public storedBytes32;

    function setup() external {
        storedUint = rvm.parseUint("123");
        storedInt = rvm.parseInt("-42");
        storedBool = rvm.parseBool("true");
        storedAddr = rvm.parseAddress("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
        storedBytes32 = rvm.parseBytes32("7465737400000000000000000000000000000000000000000000000000000000");
    }

    /// Re-parse all canonical values and overwrite storage.
    function actionReParseAll() external {
        storedUint = rvm.parseUint("123");
        storedInt = rvm.parseInt("-42");
        storedBool = rvm.parseBool("true");
        storedAddr = rvm.parseAddress("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
        storedBytes32 = rvm.parseBytes32("7465737400000000000000000000000000000000000000000000000000000000");
    }

    /// Parse a sequence of different uints, ending on the canonical value.
    function actionParseSequence() external {
        storedUint = rvm.parseUint("1");
        storedUint = rvm.parseUint("2");
        storedUint = rvm.parseUint("123");
    }

    /// Parse a different uint value, mutating the stored state.
    function actionParseDifferentUint() external {
        storedUint = rvm.parseUint("999");
    }

    /// Parse an invalid bool string. Must revert.
    function actionRevertInvalidBool() external pure {
        rvm.parseBool("maybe");
    }

    /// Parse an invalid address string. Must revert.
    function actionRevertInvalidAddress() external pure {
        rvm.parseAddress("not_an_address");
    }

    /// Parse an invalid uint string. Must revert.
    function actionRevertInvalidUint() external pure {
        rvm.parseUint("not_a_number");
    }

    /// Parse an invalid bytes32 length. Must revert.
    function actionRevertInvalidBytes32() external pure {
        rvm.parseBytes32("abcd");
    }

    /// Read the stored uint value.
    function getStoredUint() external view returns (uint256) {
        return storedUint;
    }

    /// Invariant: all stored parsed values must match their canonical values.
    function invariant_allParsedMatch() external view {
        assert(storedUint == EXPECTED_UINT);
        assert(storedInt == EXPECTED_INT);
        assert(storedBool == EXPECTED_BOOL);
        assert(storedAddr == EXPECTED_ADDR);
        assert(storedBytes32 == EXPECTED_BYTES32);
    }
}
