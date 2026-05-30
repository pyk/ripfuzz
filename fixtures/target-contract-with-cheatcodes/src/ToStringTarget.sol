// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @notice Minimal stateful-fuzzing target for raptor toString cheatcodes.
///
/// Setup converts well-known values to strings via vm.toString and stores them.
/// Actions re-convert all values; invariants verify the stored strings match.
contract ToStringTarget {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address constant TEST_ADDR = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;
    bool constant TEST_BOOL = true;
    uint256 constant TEST_UINT = 12345678901234567890;
    int256 constant TEST_INT = -12345678901234567890;
    bytes32 constant TEST_BYTES32 =
        0xabcdef0000000000000000000000000000000000000000000000000000000000;
    bytes constant TEST_BYTES = hex"deadbeef";

    string public storedAddr;
    string public storedBool;
    string public storedUint;
    string public storedInt;
    string public storedBytes32;
    string public storedBytes;

    function setup() external {
        storedAddr = vm.toString(TEST_ADDR);
        storedBool = vm.toString(TEST_BOOL);
        storedUint = vm.toString(TEST_UINT);
        storedInt = vm.toString(TEST_INT);
        storedBytes32 = vm.toString(TEST_BYTES32);
        storedBytes = vm.toString(TEST_BYTES);
    }

    /// Re-convert all canonical values and overwrite storage.
    function actionRefreshAll() external {
        storedAddr = vm.toString(TEST_ADDR);
        storedBool = vm.toString(TEST_BOOL);
        storedUint = vm.toString(TEST_UINT);
        storedInt = vm.toString(TEST_INT);
        storedBytes32 = vm.toString(TEST_BYTES32);
        storedBytes = vm.toString(TEST_BYTES);
    }

    function invariant_addr() external view {
        assert(
            keccak256(bytes(storedAddr)) ==
                keccak256(bytes("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"))
        );
    }

    function invariant_bool() external view {
        assert(keccak256(bytes(storedBool)) == keccak256(bytes("true")));
    }

    function invariant_uint() external view {
        assert(
            keccak256(bytes(storedUint)) ==
                keccak256(bytes("12345678901234567890"))
        );
    }

    function invariant_int() external view {
        assert(
            keccak256(bytes(storedInt)) ==
                keccak256(bytes("-12345678901234567890"))
        );
    }

    function invariant_bytes32() external view {
        assert(
            keccak256(bytes(storedBytes32)) ==
                keccak256(
                    bytes(
                        "0xabcdef0000000000000000000000000000000000000000000000000000000000"
                    )
                )
        );
    }

    function invariant_bytes() external view {
        assert(
            keccak256(bytes(storedBytes)) ==
                keccak256(bytes("0xdeadbeef"))
        );
    }
}
