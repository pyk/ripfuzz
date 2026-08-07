// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeFFI {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    bytes32 public recordedHash;
    uint256 public recordedTimestamp;

    // --- setup interaction ---

    function setup() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "setup";
        bytes memory res = rvm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function action_record_hash() external {
        // Re-read recordedHash so properties can assert on it
    }

    function setup_ffi_executes() external view returns (bool) {
        bytes memory expected = bytes("setup\n");
        return recordedHash == keccak256(expected);
    }

    function setup_only() external view returns (bool) {
        return recordedHash == keccak256(bytes("setup\n"));
    }

    // --- Same-sequence persistence ---

    function action_ffi_echo(string calldata msg) external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = msg;
        bytes memory res = rvm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function ffi_persists_across_calls() external view returns (bool) {
        // action_ffi_echo("hello") at idx=0, then action_record_hash() at idx=1
        // Expected: hash of "hello\n"
        return recordedHash == keccak256(bytes("hello\n"));
    }

    // --- Revert safety (host side effects are irreversible) ---

    function action_ffi_and_revert() external {
        string[] memory inputs = new string[](3);
        inputs[0] = "sh";
        inputs[1] = "-c";
        inputs[2] = "touch /tmp/ripfuzz_ffi_revert_marker";
        rvm.ffi(inputs);

        // Mutate contract state after ffi
        recordedHash = keccak256("should be reverted");
        revert("intentional");
    }

    function revert_does_not_undo_ffi() external view returns (bool) {
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
        bytes memory res = rvm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function ffi_hex_decoded() external view returns (bool) {
        bytes memory expected = abi.encode("hi");
        return recordedHash == keccak256(expected);
    }

    // --- Raw bytes fallback ---

    function action_ffi_raw() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "hello";
        bytes memory res = rvm.ffi(inputs);
        recordedHash = keccak256(res);
    }

    function ffi_raw_bytes() external view returns (bool) {
        return recordedHash == keccak256(bytes("hello\n"));
    }

    // --- Empty command reverts ---

    function action_ffi_empty() external {
        string[] memory inputs = new string[](0);
        rvm.ffi(inputs);
    }

    // --- Command failure reverts ---

    function action_ffi_fail() external {
        string[] memory inputs = new string[](1);
        inputs[0] = "false";
        rvm.ffi(inputs);
    }

    // --- Property sees final FFI result ---

    function final_ffi_result() external view returns (bool) {
        // If the only call was action_ffi_echo("final"), the hash should match
        return recordedHash == keccak256(bytes("final\n"));
    }

    // --- Cross-cheatcode interaction: FFI + warp ---

    function action_ffi_and_warp() external {
        string[] memory inputs = new string[](2);
        inputs[0] = "echo";
        inputs[1] = "gm";
        bytes memory res = rvm.ffi(inputs);
        recordedHash = keccak256(res);

        rvm.warp(999999);
        recordedTimestamp = block.timestamp;
    }

    function ffi_and_warp() external view returns (bool) {
        return recordedHash == keccak256(bytes("gm\n"))
            && recordedTimestamp == 999999;
    }
}