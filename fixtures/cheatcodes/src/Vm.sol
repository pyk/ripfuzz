// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface Vm {
    // State / Block manipulation
    function warp(uint256) external;
    function roll(uint256) external;
    function fee(uint256) external;
    function coinbase(address) external;
    function difficulty(uint256) external;
    function prevrandao(bytes32) external;
    function chainId(uint256) external;

    // Account manipulation
    function deal(address, uint256) external;
    function etch(address, bytes calldata) external;
    function setNonce(address, uint64) external;
    function getNonce(address) external view returns (uint64);
    function load(address, bytes32) external view returns (bytes32);
    function store(address, bytes32, bytes32) external;

    // Labeling
    function label(address, string calldata) external;
    function getLabel(address) external view returns (string memory);

    // Prank
    function prank(address) external;
    function prank(address, address) external;
    function startPrank(address) external;
    function startPrank(address, address) external;
    function stopPrank() external;

    // Assertions
    function assertTrue(bool) external;
    function assertFalse(bool) external;
    function assertEq(bool, bool) external;
    function assertEq(uint256, uint256) external;
    function assertEq(int256, int256) external;
    function assertEq(address, address) external;
    function assertEq(bytes32, bytes32) external;
    function assertEq(string calldata, string calldata) external;
    function assertEq(bytes calldata, bytes calldata) external;
    function assertNotEq(bool, bool) external;
    function assertNotEq(uint256, uint256) external;
    function assertNotEq(int256, int256) external;
    function assertNotEq(address, address) external;
    function assertNotEq(bytes32, bytes32) external;
    function assertNotEq(string calldata, string calldata) external;
    function assertNotEq(bytes calldata, bytes calldata) external;
    function assertLt(uint256, uint256) external;
    function assertLt(int256, int256) external;
    function assertLe(uint256, uint256) external;
    function assertLe(int256, int256) external;
    function assertGt(uint256, uint256) external;
    function assertGt(int256, int256) external;
    function assertGe(uint256, uint256) external;
    function assertGe(int256, int256) external;

    // String / Type conversion
    function toString(address) external pure returns (string memory);
    function toString(bool) external pure returns (string memory);
    function toString(uint256) external pure returns (string memory);
    function toString(int256) external pure returns (string memory);
    function toString(bytes32) external pure returns (string memory);
    function toString(bytes calldata) external pure returns (string memory);
    function parseUint(string calldata) external pure returns (uint256);
    function parseInt(string calldata) external pure returns (int256);
    function parseBool(string calldata) external pure returns (bool);
    function parseAddress(string calldata) external pure returns (address);
    function parseBytes(string calldata) external pure returns (bytes memory);
    function parseBytes32(string calldata) external pure returns (bytes32);
    function getCode(string calldata) external view returns (bytes memory);

    // Wallet / Crypto
    function addr(uint256) external pure returns (address);
    function sign(uint256, bytes32) external pure returns (uint8, bytes32, bytes32);

    // FFI
    function ffi(string[] calldata) external returns (bytes memory);
}
