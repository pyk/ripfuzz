// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {Challenge, Finding, Severity} from "./Challenge.sol";

/// @title GatedByLiterals
/// @custom:level Easy
/// @dev Every handler reports a finding behind a comparison with
///      a literal.
///
///      The fuzzer must extract the literals from this source and
///      use them as arguments to reach every gate.
///
///      One gate per literal kind:
///      - `bool`
///      - `uint256`
///      - `uint128`
///      - `int256`
///      - `int8`
///      - `bytes32`
///      - `bytes1`
///      - `address`
///      - `bytes`
///      - `string`
///      - the `1 ether` subdenomination
contract GatedByLiterals is Challenge {
    function gatedByBoolLiteral(bool flag) external {
        if (flag == true) {
            rvm.finding(
                Finding({
                    id: "GATED-BOOL",
                    severity: Severity.Medium,
                    title: "gated by bool literal",
                    description: "flag == true"
                })
            );
        }
    }

    function gatedByUint256Literal(uint256 value) external {
        if (value == 2) {
            rvm.finding(
                Finding({
                    id: "GATED-UINT256",
                    severity: Severity.Medium,
                    title: "gated by uint256 literal",
                    description: "value == 2"
                })
            );
        }
    }

    function gatedByUint128Literal(uint128 value) external {
        if (value == 12345) {
            rvm.finding(
                Finding({
                    id: "GATED-UINT128",
                    severity: Severity.Medium,
                    title: "gated by uint128 literal",
                    description: "value == 12345"
                })
            );
        }
    }

    function gatedByInt256Literal(int256 value) external {
        if (value == -7) {
            rvm.finding(
                Finding({
                    id: "GATED-INT256",
                    severity: Severity.Medium,
                    title: "gated by int256 literal",
                    description: "value == -7"
                })
            );
        }
    }

    function gatedByInt8Literal(int8 value) external {
        if (value == -3) {
            rvm.finding(
                Finding({
                    id: "GATED-INT8",
                    severity: Severity.Medium,
                    title: "gated by int8 literal",
                    description: "value == -3"
                })
            );
        }
    }

    function gatedByBytes32Literal(bytes32 hash) external {
        if (hash == bytes32(0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef)) {
            rvm.finding(
                Finding({
                    id: "GATED-BYTES32",
                    severity: Severity.Medium,
                    title: "gated by bytes32 literal",
                    description: "hash == 0x123456..."
                })
            );
        }
    }

    function gatedByBytes1Literal(bytes1 tag) external {
        if (tag == bytes1(0xab)) {
            rvm.finding(
                Finding({
                    id: "GATED-BYTES1",
                    severity: Severity.Medium,
                    title: "gated by bytes1 literal",
                    description: "tag == 0xab"
                })
            );
        }
    }

    function gatedByAddressLiteral(address account) external {
        if (account == 0x5B38Da6a701c568545dCfcB03FcB875f56beddC4) {
            rvm.finding(
                Finding({
                    id: "GATED-ADDRESS",
                    severity: Severity.Medium,
                    title: "gated by address literal",
                    description: "account == 0x5B38..."
                })
            );
        }
    }

    function gatedByBytesLiteral(bytes memory data) external {
        if (keccak256(data) == keccak256(hex"deadbeef")) {
            rvm.finding(
                Finding({
                    id: "GATED-BYTES",
                    severity: Severity.Medium,
                    title: "gated by bytes literal",
                    description: "keccak256(data) == keccak256(0xdeadbeef)"
                })
            );
        }
    }

    function gatedByStringLiteral(string memory text) external {
        if (keccak256(bytes(text)) == keccak256(bytes("gold"))) {
            rvm.finding(
                Finding({
                    id: "GATED-STRING",
                    severity: Severity.Medium,
                    title: "gated by string literal",
                    description: "text == gold"
                })
            );
        }
    }

    function gatedByEtherLiteral(uint256 value) external {
        if (value == 1 ether) {
            rvm.finding(
                Finding({
                    id: "GATED-ETHER",
                    severity: Severity.Medium,
                    title: "gated by ether literal",
                    description: "value == 1 ether"
                })
            );
        }
    }
}
