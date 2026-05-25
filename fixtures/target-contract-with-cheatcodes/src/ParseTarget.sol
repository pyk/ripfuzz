// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract ParseTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_UINT = 123;
    int256 constant EXPECTED_INT = -42;
    bool constant EXPECTED_BOOL = true;
    address constant EXPECTED_ADDR =
        0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    bytes32 constant EXPECTED_BYTES32 =
        0x7465737400000000000000000000000000000000000000000000000000000000;

    uint256 public storedUint;
    int256 public storedInt;
    bool public storedBool;
    address public storedAddr;
    bytes32 public storedBytes32;

    function setup() external {
        storedUint = vm.parseUint("123");
        storedInt = vm.parseInt("-42");
        storedBool = vm.parseBool("true");
        storedAddr = vm.parseAddress(
            "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF"
        );
        storedBytes32 = vm.parseBytes32(
            "7465737400000000000000000000000000000000000000000000000000000000"
        );
    }

    /// Re-parse all canonical values and overwrite storage.
    function actionReParseAll() external {
        storedUint = vm.parseUint("123");
        storedInt = vm.parseInt("-42");
        storedBool = vm.parseBool("true");
        storedAddr = vm.parseAddress(
            "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF"
        );
        storedBytes32 = vm.parseBytes32(
            "7465737400000000000000000000000000000000000000000000000000000000"
        );
    }

    /// Parse a sequence of different uints, ending on the canonical value.
    function actionParseSequence() external {
        storedUint = vm.parseUint("1");
        storedUint = vm.parseUint("2");
        storedUint = vm.parseUint("123");
    }

    /// Parse a different uint value, mutating the stored state.
    function actionParseDifferentUint() external {
        storedUint = vm.parseUint("999");
    }

    /// Parse an invalid bool string. Must revert.
    function actionRevertInvalidBool() external pure {
        vm.parseBool("maybe");
    }

    /// Parse an invalid address string. Must revert.
    function actionRevertInvalidAddress() external pure {
        vm.parseAddress("not_an_address");
    }

    /// Parse an invalid uint string. Must revert.
    function actionRevertInvalidUint() external pure {
        vm.parseUint("not_a_number");
    }

    /// Parse an invalid bytes32 length. Must revert.
    function actionRevertInvalidBytes32() external pure {
        vm.parseBytes32("abcd");
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
