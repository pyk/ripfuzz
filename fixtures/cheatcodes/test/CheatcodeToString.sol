// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

contract CheatcodeToString {
    RVM constant rvm = RVM(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    // --- Setup state ---
    string public setupUint;
    string public setupBool;
    string public setupAddress;
    string public setupBytes32;
    string public setupInt;
    string public setupBytes;

    // --- Edge-case state ---
    string public edgeUint;
    string public edgeMaxUint;
    string public edgeInt;
    string public edgeAddress;
    string public edgeBool;
    string public edgeBytes;
    string public edgeBytes32;

    // --- Round-trip state ---
    string public rtUint;
    string public rtInt;
    string public rtAddress;
    string public rtBool;
    string public rtBytes32;
    string public rtBytes;

    // --- Side-effect state ---
    uint256 public warpTs;
    uint256 public rollNum;

    function setup() external {
        setupUint = rvm.toString(uint256(123));
        setupBool = rvm.toString(true);
        setupAddress = rvm.toString(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
        setupBytes32 = rvm.toString(bytes32(uint256(0xdeadbeef)));
        setupInt = rvm.toString(int256(-42));
        setupBytes = rvm.toString(bytes(hex"01ab"));
    }

    // --- Setup properties ---

    function check_setupUint() external view returns (bool) {
        return keccak256(bytes(setupUint)) == keccak256(bytes("123"));
    }

    function check_setupBool() external view returns (bool) {
        return keccak256(bytes(setupBool)) == keccak256(bytes("true"));
    }

    function check_setupAddress() external view returns (bool) {
        return keccak256(bytes(setupAddress))
            == keccak256(bytes("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"));
    }

    function check_setupBytes32() external view returns (bool) {
        return keccak256(bytes(setupBytes32))
            == keccak256(bytes("0x00000000000000000000000000000000000000000000000000000000deadbeef"));
    }

    function check_setupInt() external view returns (bool) {
        return keccak256(bytes(setupInt)) == keccak256(bytes("-42"));
    }

    function check_setupBytes() external view returns (bool) {
        return keccak256(bytes(setupBytes)) == keccak256(bytes("0x01ab"));
    }

    // --- Round-trip actions ---

    function action_roundTripUint(uint256 value) external {
        string memory s = rvm.toString(value);
        require(rvm.parseUint(s) == value, "round-trip uint failed");
        rtUint = s;
    }

    function action_roundTripInt(int256 value) external {
        string memory s = rvm.toString(value);
        require(rvm.parseInt(s) == value, "round-trip int failed");
        rtInt = s;
    }

    function action_roundTripAddress(address value) external {
        string memory s = rvm.toString(value);
        require(rvm.parseAddress(s) == value, "round-trip address failed");
        rtAddress = s;
    }

    function action_roundTripBool(bool value) external {
        string memory s = rvm.toString(value);
        require(rvm.parseBool(s) == value, "round-trip bool failed");
        rtBool = s;
    }

    function action_roundTripBytes32(bytes32 value) external {
        string memory s = rvm.toString(value);
        require(rvm.parseBytes32(s) == value, "round-trip bytes32 failed");
        rtBytes32 = s;
    }

    function action_roundTripBytes(bytes calldata value) external {
        string memory s = rvm.toString(value);
        bytes memory parsed = rvm.parseBytes(s);
        require(keccak256(parsed) == keccak256(value), "round-trip bytes failed");
        rtBytes = s;
    }

    // --- Round-trip properties ---

    function check_rtUint() external view returns (bool) {
        return keccak256(bytes(rtUint)) == keccak256(bytes("12345"));
    }

    function check_rtInt() external view returns (bool) {
        return keccak256(bytes(rtInt)) == keccak256(bytes("-123"));
    }

    function check_rtAddress() external view returns (bool) {
        return keccak256(bytes(rtAddress))
            == keccak256(bytes("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D"));
    }

    function check_rtBool() external view returns (bool) {
        return keccak256(bytes(rtBool)) == keccak256(bytes("true"));
    }

    function check_rtBytes32() external view returns (bool) {
        return keccak256(bytes(rtBytes32))
            == keccak256(bytes("0xdeadbeef00000000000000000000000000000000000000000000000000000000"));
    }

    function check_rtBytes() external view returns (bool) {
        return keccak256(bytes(rtBytes)) == keccak256(bytes("0x01ab"));
    }

    // --- Edge-case actions ---

    function action_toStringZeroUint() external {
        edgeUint = rvm.toString(uint256(0));
    }

    function action_toStringMaxUint() external {
        edgeMaxUint = rvm.toString(type(uint256).max);
    }

    function action_toStringMinInt() external {
        edgeInt = rvm.toString(type(int256).min);
    }

    function action_toStringZeroAddress() external {
        edgeAddress = rvm.toString(address(0));
    }

    function action_toStringFalse() external {
        edgeBool = rvm.toString(false);
    }

    function action_toStringEmptyBytes() external {
        edgeBytes = rvm.toString(bytes(""));
    }

    function action_toStringEmptyBytes32() external {
        edgeBytes32 = rvm.toString(bytes32(0));
    }

    // --- Edge-case properties ---

    function check_edgeUint() external view returns (bool) {
        return keccak256(bytes(edgeUint)) == keccak256(bytes("0"));
    }

    function check_edgeMaxUint() external view returns (bool) {
        return keccak256(bytes(edgeMaxUint))
            == keccak256(bytes("115792089237316195423570985008687907853269984665640564039457584007913129639935"));
    }

    function check_edgeInt() external view returns (bool) {
        return keccak256(bytes(edgeInt))
            == keccak256(bytes("-57896044618658097711785492504343953926634992332820282019728792003956564819968"));
    }

    function check_edgeAddress() external view returns (bool) {
        return keccak256(bytes(edgeAddress))
            == keccak256(bytes("0x0000000000000000000000000000000000000000"));
    }

    function check_edgeBool() external view returns (bool) {
        return keccak256(bytes(edgeBool)) == keccak256(bytes("false"));
    }

    function check_edgeBytes() external view returns (bool) {
        return keccak256(bytes(edgeBytes)) == keccak256(bytes("0x"));
    }

    function check_edgeBytes32() external view returns (bool) {
        return keccak256(bytes(edgeBytes32))
            == keccak256(bytes("0x0000000000000000000000000000000000000000000000000000000000000000"));
    }

    // --- Side-effect isolation ---

    function action_toStringThenWarp() external {
        rvm.toString(uint256(42));
        rvm.warp(1234567890);
        warpTs = block.timestamp;
    }

    function action_toStringThenRoll() external {
        rvm.toString(uint256(42));
        rvm.roll(9999);
        rollNum = block.number;
    }

    function check_sideEffects() external view returns (bool) {
        return warpTs == 1234567890 && rollNum == 9999;
    }

    // --- Same-sequence independence ---

    function action_twoToStringCalls() external {
        string memory s1 = rvm.toString(uint256(1));
        string memory s2 = rvm.toString(uint256(2));
        require(keccak256(bytes(s1)) == keccak256(bytes("1")), "first toString corrupted");
        require(keccak256(bytes(s2)) == keccak256(bytes("2")), "second toString corrupted");
    }

    function check_twoToStringCalls() external pure returns (bool) {
        return true;
    }
}
