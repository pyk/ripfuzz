// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract ParseTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 public constant EXPECTED_UINT = 123;
    int256 public constant EXPECTED_INT = -42;
    bool public constant EXPECTED_BOOL = true;
    address public constant EXPECTED_ADDR =
        0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    bytes32 public constant EXPECTED_BYTES32 =
        0x7465737400000000000000000000000000000000000000000000000000000000;

    uint256 public storedUint;
    int256 public storedInt;
    bool public storedBool;
    address public storedAddr;
    bytes public storedBytes;
    bytes32 public storedBytes32;

    function setup() external {
        storedUint = vm.parseUint("123");
        storedInt = vm.parseInt("-42");
        storedBool = vm.parseBool("true");
        storedAddr = vm.parseAddress(
            "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF"
        );
        storedBytes = vm.parseBytes("0x1234abcd");
        storedBytes32 = vm.parseBytes32(
            "7465737400000000000000000000000000000000000000000000000000000000"
        );
    }

    // -----------------------------------------------------------------
    // Getters
    // -----------------------------------------------------------------

    function getParsedUint() external view returns (uint256) {
        return storedUint;
    }

    function getParsedInt() external view returns (int256) {
        return storedInt;
    }

    function getParsedBool() external view returns (bool) {
        return storedBool;
    }

    function getParsedAddress() external view returns (address) {
        return storedAddr;
    }

    function getParsedBytes() external view returns (bytes memory) {
        return storedBytes;
    }

    function getParsedBytes32() external view returns (bytes32) {
        return storedBytes32;
    }

    // -----------------------------------------------------------------
    // Single-transaction determinism
    // -----------------------------------------------------------------

    function callParseUintSameValueTwice()
        external
        pure
        returns (uint256 first, uint256 second)
    {
        first = vm.parseUint("456");
        second = vm.parseUint("456");
    }

    function callParseBytes32SameValueTwice()
        external
        pure
        returns (bytes32 first, bytes32 second)
    {
        first = vm.parseBytes32(
            "abababababababababababababababababababababababababababababababab"
        );
        second = vm.parseBytes32(
            "abababababababababababababababababababababababababababababababab"
        );
    }

    // -----------------------------------------------------------------
    // Sequence independence
    // -----------------------------------------------------------------

    function callParseUintSequence()
        external
        pure
        returns (uint256 first, uint256 second, uint256 third)
    {
        first = vm.parseUint("1");
        second = vm.parseUint("2");
        third = vm.parseUint("1");
    }

    function callParseBoolSequence()
        external
        pure
        returns (bool first, bool second, bool third)
    {
        first = vm.parseBool("true");
        second = vm.parseBool("false");
        third = vm.parseBool("true");
    }

    // -----------------------------------------------------------------
    // Edge cases - must revert
    // -----------------------------------------------------------------

    function parseInvalidBool() external pure {
        vm.parseBool("maybe");
    }

    function parseInvalidAddress() external pure {
        vm.parseAddress("not_an_address");
    }

    function parseInvalidBytes32Length() external pure {
        vm.parseBytes32("abcd");
    }

    function parseInvalidUint() external pure {
        vm.parseUint("not_a_number");
    }

    // -----------------------------------------------------------------
    // Interaction with other cheatcodes
    // -----------------------------------------------------------------

    function callParseAndDeal()
        external
        returns (uint256 parsed, uint256 balance)
    {
        parsed = vm.parseUint("1000");
        vm.deal(address(this), parsed);
        balance = address(this).balance;
    }

    function callParseAndWarp()
        external
        returns (uint256 parsed, uint256 timestamp)
    {
        parsed = vm.parseUint("1234567890");
        vm.warp(parsed);
        timestamp = block.timestamp;
    }

    function callParseAndChainId()
        external
        returns (uint256 parsed, uint256 chainId)
    {
        parsed = vm.parseUint("99");
        vm.chainId(parsed);
        chainId = block.chainid;
    }

    // -----------------------------------------------------------------
    // Fuzzing actions
    // -----------------------------------------------------------------

    function actionParseUint() external {
        storedUint = vm.parseUint("123");
    }

    function actionParseBytes32() external {
        storedBytes32 = vm.parseBytes32(
            "7465737400000000000000000000000000000000000000000000000000000000"
        );
    }

    function actionParseAndDeal() external {
        uint256 value = vm.parseUint("1000");
        vm.deal(address(this), value);
    }

    function getBalance() external view returns (uint256) {
        return address(this).balance;
    }

    // -----------------------------------------------------------------
    // Invariants
    // -----------------------------------------------------------------

    function invariant_parsed_uint() external view {
        assert(storedUint == EXPECTED_UINT);
    }

    function invariant_parsed_int() external view {
        assert(storedInt == EXPECTED_INT);
    }

    function invariant_parsed_bool() external view {
        assert(storedBool == EXPECTED_BOOL);
    }

    function invariant_parsed_address() external view {
        assert(storedAddr == EXPECTED_ADDR);
    }

    function invariant_parsed_bytes32() external view {
        assert(storedBytes32 == EXPECTED_BYTES32);
    }
}
