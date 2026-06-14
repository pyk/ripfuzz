// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract LiteralHandler {
    // State variables used by literal groups
    bool public flag;
    uint256 public num;
    string public message;
    bytes32 public data;
    address public target;

    // --- Bool literals ---
    function useBools() external {
        flag = true;
        flag = false;
    }

    // --- Number literals (plain integers) ---
    function useNumbers() external {
        num = 0;
        num = 1;
        num = 42;
        num = 1000;
        num = 1337;
    }

    // --- Number literal formats ---
    function useNumberFormats() external {
        num = 0x1234; // hex number
        num = 1e18; // scientific notation
        num = 1_000_000; // underscore separators
        num = 115792089237316195423570985008687907853269984665640564039457584007913129639935; // uint256 max
    }

    // --- Signed number literals ---
    function useSignedNumbers() external {
        int256 snum = -1;
        snum = -42;
        snum = -128;
        snum = -129;
    }

    // --- Number literals with subdenominations ---
    function useSubdenominations() external {
        // Currency units
        num = 1 wei;
        num = 100 wei;
        num = 1 gwei;
        num = 1 ether;
        num = 0.5 ether;

        // Time units
        num = 5 seconds;
        num = 1 minutes;
        num = 1 hours;
        num = 2 days;
        num = 1 weeks;
    }

    // --- String literals ---
    function useStrings() external {
        message = ""; // empty
        message = "hello";
        message = "world";
        message = "ok";
        message = "hello\nworld"; // escape sequence
    }

    // --- Hex string literals ---
    function useHexStrings() external {
        data = hex""; // empty
        data = hex"00"; // short
        data = hex"1234"; // single quotes
        data = hex"1234567890abcdef1234567890abcdef12345678";
        data = hex"deadbeef00000000000000000000000000000000000000000000000000000000";
    }

    // --- Unicode string literals ---
    function useUnicodeStrings() external {
        message = unicode""; // empty
        message = unicode"hello 🌍";
    }

    // --- Address literals ---
    function useAddresses() external {
        target = 0xabCDEF1234567890ABcDEF1234567890aBCDeF12;
    }

    function invariant_check() public view {
        require(num >= 0, "ok");
    }
}
