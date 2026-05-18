// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeFFI {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    bytes32 public recordedHash;
    uint256 public recordedTimestamp;

    // --- setUp interaction ---

    function setUp() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "setup";
        bytes memory res = vm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function action_record_hash() external {
        // Re-read recordedHash so properties can assert on it
    }

    function property_setup_ffi_executes() external view returns (bool) {
        bytes memory expected = bytes("setup\n");
        return recordedHash == keccak256(expected);
    }

    function property_setup_only() external view returns (bool) {
        return recordedHash == keccak256(bytes("setup\n"));
    }

    // --- Same-sequence persistence ---

    function action_ffi_echo(string calldata msg) external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = msg;
        bytes memory res = vm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function property_ffi_persists_across_calls() external view returns (bool) {
        // action_ffi_echo("hello") at idx=0, then action_record_hash() at idx=1
        // Expected: hash of "hello\n"
        return recordedHash == keccak256(bytes("hello\n"));
    }

    // --- Revert safety (host side effects are irreversible) ---

    function action_ffi_and_revert() external {
        string[] memory inputs = new string[](3);
        inputs[0] = "sh";
        inputs[1] = "-c";
        inputs[2] = "touch /tmp/raptor_ffi_revert_marker";
        vm.ffi(inputs);

        // Mutate contract state after ffi
        recordedHash = keccak256("should be reverted");
        revert("intentional");
    }

    function property_revert_does_not_undo_ffi() external view returns (bool) {
        // The storage write after ffi was reverted, so recordedHash should
        // still be the setup value (or whatever it was before the reverted call).
        return recordedHash == keccak256(bytes("setup\n"));
    }

    // --- Hex decoding ---

    function action_ffi_hex() external {
        string[] memory inputs = new string[](3);
        inputs[0] = "printf";
        inputs[1] = "%s";
        // ABI-encoded "hi" as hex string with 0x prefix
        inputs[2] = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000026869000000000000000000000000000000000000000000000000000000000000";
        bytes memory res = vm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function property_ffi_hex_decoded() external view returns (bool) {
        bytes memory expected = abi.encode("hi");
        return recordedHash == keccak256(expected);
    }

    // --- Raw bytes fallback ---

    function action_ffi_raw() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "hello";
        bytes memory res = vm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function property_ffi_raw_bytes() external view returns (bool) {
        return recordedHash == keccak256(bytes("hello\n"));
    }

    // --- Empty command reverts ---

    function action_ffi_empty() external {
        string[] memory inputs = new string[](0);
        vm.ffi(inputs);
    }

    // --- Command failure reverts ---

    function action_ffi_fail() external {
        string[] memory inputs = new string[](1);
        inputs[0] = "false";
        vm.ffi(inputs);
    }

    // --- Property sees final FFI result ---

    function property_final_ffi_result() external view returns (bool) {
        // If the only call was action_ffi_echo("final"), the hash should match
        return recordedHash == keccak256(bytes("final\n"));
    }

    // --- Cross-cheatcode interaction: FFI + warp ---

    function action_ffi_and_warp() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "gm";
        bytes memory res = vm.ffi(inputs);
        recordedHash = keccak256(res);

        vm.warp(999999);
        recordedTimestamp = block.timestamp;
    }

    function property_ffi_and_warp() external view returns (bool) {
        return recordedHash == keccak256(bytes("gm\n"))
            && recordedTimestamp == 999999;
    }
}