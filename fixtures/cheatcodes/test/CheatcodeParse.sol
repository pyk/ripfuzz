// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeParse {
    RVM constant rvm = RVM(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    // State variables to persist parse results across calls
    uint256 public storedUint;
    int256 public storedInt;
    bool public storedBool;
    address public storedAddr;
    bytes public storedBytes;
    bytes32 public storedBytes32;

    // --- setup interaction ---

    function setup() external {
        storedUint = rvm.parseUint("999");
        storedInt = rvm.parseInt("-999");
        storedBool = rvm.parseBool("true");
        storedAddr = rvm.parseAddress(
            "0x71C7656EC7ab88b098defB751B7401B5f6d8976F"
        );
        storedBytes = rvm.parseBytes("0xabcd");
        storedBytes32 = rvm.parseBytes32(
            "0x1111111111111111111111111111111111111111111111111111111111111111"
        );
    }

    function setup_parse_persists() external view returns (bool) {
        return
            storedUint == 999 &&
            storedInt == -999 &&
            storedBool == true &&
            storedAddr == address(0x71C7656EC7ab88b098defB751B7401B5f6d8976F) &&
            keccak256(storedBytes) == keccak256(hex"abcd") &&
            storedBytes32 ==
            bytes32(
                hex"1111111111111111111111111111111111111111111111111111111111111111"
            );
    }

    // --- Same-sequence calls ---

    function call_parse_and_store(string calldata s) external {
        storedUint = rvm.parseUint(s);
    }

    function parse_persists_in_sequence()
        external
        view
        returns (bool)
    {
        // call_parse_and_store("42") followed by this property should see 42
        return storedUint == 42;
    }

    // --- Pure isolation (parse does not mutate DB) ---

    function call_parse_no_side_effect(string calldata s) external view {
        // Pure parse should not change any chain state
        rvm.parseUint(s);
        rvm.parseInt(s);
        rvm.parseBool("true");
        rvm.parseAddress("0x71C7656EC7ab88b098defB751B7401B5f6d8976F");
        rvm.parseBytes("0x00");
        rvm.parseBytes32(
            "0x1111111111111111111111111111111111111111111111111111111111111111"
        );
    }

    function pure_isolation() external view returns (bool) {
        // After call_parse_no_side_effect, nothing should have changed
        return storedUint == 999; // setup value intact
    }

    // --- Revert safety ---

    function call_parse_and_revert(string calldata badUint) external {
        rvm.parseUint(badUint); // if badUint is malformed, this reverts
        revert("should not reach here");
    }

    function revert_on_malformed() external view returns (bool) {
        // setup value should still be intact because the call reverted
        return storedUint == 999;
    }

    // --- Cross-cheatcode: parse + deal ---

    function call_parse_then_deal(string calldata amtStr) external {
        uint256 amt = rvm.parseUint(amtStr);
        rvm.deal(address(0xBEEF), amt);
    }

    function parse_deal() external view returns (bool) {
        return address(0xBEEF).balance == 1000;
    }

    // --- Round-trip: toString -> parse ---

    function to_string_round_trip() external view returns (bool) {
        uint256 original = 12345;
        uint256 recovered = rvm.parseUint(rvm.toString(original));
        return recovered == original;
    }

    function bool_round_trip() external view returns (bool) {
        bool original = true;
        bool recovered = rvm.parseBool(rvm.toString(original));
        return recovered == original;
    }

    // --- Edge: max values ---

    function max_uint() external view returns (bool) {
        uint256 max = type(uint256).max;
        uint256 parsed = rvm.parseUint(rvm.toString(max));
        return parsed == max;
    }

    function max_int() external view returns (bool) {
        int256 max = type(int256).max;
        int256 parsed = rvm.parseInt(rvm.toString(max));
        return parsed == max;
    }

    function min_int() external view returns (bool) {
        int256 min = type(int256).min;
        int256 parsed = rvm.parseInt(rvm.toString(min));
        return parsed == min;
    }

    // --- Edge: hex inputs ---

    function hex_uint() external view returns (bool) {
        return rvm.parseUint("0xff") == 255;
    }

    function hex_address() external view returns (bool) {
        return
            rvm.parseAddress("0x71c7656ec7ab88b098defb751b7401b5f6d8976f") ==
            address(0x71C7656EC7ab88b098defB751B7401B5f6d8976F);
    }

    function hex_bytes32() external view returns (bool) {
        return
            rvm.parseBytes32(
                "0x2222222222222222222222222222222222222222222222222222222222222222"
            ) ==
            bytes32(
                hex"2222222222222222222222222222222222222222222222222222222222222222"
            );
    }

    // --- Edge: bool variants ---

    function bool_true_variants() external view returns (bool) {
        return
            rvm.parseBool("true") &&
            rvm.parseBool("TRUE") &&
            rvm.parseBool("True") &&
            rvm.parseBool("1");
    }

    function bool_false_variants() external view returns (bool) {
        return
            !rvm.parseBool("false") &&
            !rvm.parseBool("FALSE") &&
            !rvm.parseBool("False") &&
            !rvm.parseBool("0");
    }
}
