// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract ToStringTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant TEST_ADDR = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf;
    bool constant TEST_BOOL = true;
    uint256 constant TEST_UINT = 12345678901234567890;
    int256 constant TEST_INT = -12345678901234567890;
    bytes32 constant TEST_BYTES32 =
        0xabcdef0000000000000000000000000000000000000000000000000000000000;
    bytes constant TEST_BYTES = hex"deadbeef";

    string public storedAddrString;
    string public storedBoolString;
    string public storedUintString;
    string public storedIntString;
    string public storedBytes32String;
    string public storedBytesString;

    function setup() external {
        storedAddrString = vm.toString(TEST_ADDR);
        storedBoolString = vm.toString(TEST_BOOL);
        storedUintString = vm.toString(TEST_UINT);
        storedIntString = vm.toString(TEST_INT);
        storedBytes32String = vm.toString(TEST_BYTES32);
        storedBytesString = vm.toString(TEST_BYTES);
    }

    function getStoredAddrString() external view returns (string memory s) {
        s = storedAddrString;
    }

    function getStoredBoolString() external view returns (string memory s) {
        s = storedBoolString;
    }

    function getStoredUintString() external view returns (string memory s) {
        s = storedUintString;
    }

    function getStoredIntString() external view returns (string memory s) {
        s = storedIntString;
    }

    function getStoredBytes32String() external view returns (string memory s) {
        s = storedBytes32String;
    }

    function getStoredBytesString() external view returns (string memory s) {
        s = storedBytesString;
    }

    // -----------------------------------------------------------------
    // Edge-case getters
    // -----------------------------------------------------------------

    function getAddrZeroString() external pure returns (string memory s) {
        s = vm.toString(address(0));
    }

    function getBoolFalseString() external pure returns (string memory s) {
        s = vm.toString(false);
    }

    function getUintZeroString() external pure returns (string memory s) {
        s = vm.toString(uint256(0));
    }

    function getIntZeroString() external pure returns (string memory s) {
        s = vm.toString(int256(0));
    }

    function getIntNegativeOneString() external pure returns (string memory s) {
        s = vm.toString(int256(-1));
    }

    function getIntMinString() external pure returns (string memory s) {
        s = vm.toString(type(int256).min);
    }

    function getUintMaxString() external pure returns (string memory s) {
        s = vm.toString(type(uint256).max);
    }

    function getBytes32ZeroString() external pure returns (string memory s) {
        s = vm.toString(bytes32(0));
    }

    function getBytesEmptyString() external pure returns (string memory s) {
        s = vm.toString(bytes(""));
    }

    // -----------------------------------------------------------------
    // Single-transaction determinism
    // -----------------------------------------------------------------

    function callToStringAddrSameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(TEST_ADDR);
        second = vm.toString(TEST_ADDR);
    }

    function callToStringBoolSameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(true);
        second = vm.toString(true);
    }

    function callToStringUintSameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(TEST_UINT);
        second = vm.toString(TEST_UINT);
    }

    function callToStringIntSameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(TEST_INT);
        second = vm.toString(TEST_INT);
    }

    function callToStringBytes32SameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(TEST_BYTES32);
        second = vm.toString(TEST_BYTES32);
    }

    function callToStringBytesSameValueTwice()
        external
        pure
        returns (string memory first, string memory second)
    {
        first = vm.toString(TEST_BYTES);
        second = vm.toString(TEST_BYTES);
    }

    // -----------------------------------------------------------------
    // Sequence independence
    // -----------------------------------------------------------------

    function callToStringUintSequence()
        external
        pure
        returns (string memory first, string memory second, string memory third)
    {
        first = vm.toString(uint256(1));
        second = vm.toString(TEST_UINT);
        third = vm.toString(uint256(1));
    }

    function callToStringBoolSequence()
        external
        pure
        returns (string memory first, string memory second, string memory third)
    {
        first = vm.toString(true);
        second = vm.toString(false);
        third = vm.toString(true);
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    function callToStringAndWarp()
        external
        returns (string memory addrStr, uint256 timestamp)
    {
        addrStr = vm.toString(TEST_ADDR);
        vm.warp(1234567890);
        timestamp = block.timestamp;
    }

    function callToStringAndDeal()
        external
        returns (string memory addrStr, uint256 balance)
    {
        addrStr = vm.toString(TEST_ADDR);
        vm.deal(address(this), 5 ether);
        balance = address(this).balance;
    }

    // -----------------------------------------------------------------
    // Fuzzing actions
    // -----------------------------------------------------------------

    function actionToStringAddr() external {
        storedAddrString = vm.toString(TEST_ADDR);
    }

    function actionToStringBool() external {
        storedBoolString = vm.toString(TEST_BOOL);
    }

    function actionToStringUint() external {
        storedUintString = vm.toString(TEST_UINT);
    }

    function actionToStringInt() external {
        storedIntString = vm.toString(TEST_INT);
    }

    function actionToStringBytes32() external {
        storedBytes32String = vm.toString(TEST_BYTES32);
    }

    function actionToStringBytes() external {
        storedBytesString = vm.toString(TEST_BYTES);
    }

    // -----------------------------------------------------------------
    // Invariants
    // -----------------------------------------------------------------

    function invariant_to_string_addr() external view {
        assert(
            keccak256(bytes(storedAddrString)) ==
                keccak256(bytes("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"))
        );
    }

    function invariant_to_string_bool() external view {
        assert(keccak256(bytes(storedBoolString)) == keccak256(bytes("true")));
    }

    function invariant_to_string_uint() external view {
        assert(
            keccak256(bytes(storedUintString)) ==
                keccak256(bytes("12345678901234567890"))
        );
    }

    function invariant_to_string_int() external view {
        assert(
            keccak256(bytes(storedIntString)) ==
                keccak256(bytes("-12345678901234567890"))
        );
    }

    function invariant_to_string_bytes32() external view {
        assert(
            keccak256(bytes(storedBytes32String)) ==
                keccak256(
                    bytes(
                        "0xabcdef0000000000000000000000000000000000000000000000000000000000"
                    )
                )
        );
    }

    function invariant_to_string_bytes() external view {
        assert(
            keccak256(bytes(storedBytesString)) ==
                keccak256(bytes("0xdeadbeef"))
        );
    }
}
