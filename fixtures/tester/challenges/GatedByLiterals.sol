// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {BrokenInvariantError} from "./Challenge.sol";

/// @title GatedByLiterals
/// @custom:level Easy
/// @dev Every handler reports a broken invariant behind a comparison with
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
contract GatedByLiterals {
    function gatedByBoolLiteral(bool flag) external {
        if (flag == true) {
            revert BrokenInvariantError({id: "GATED-BOOL", description: "flag == true"});
        }
    }

    function gatedByUint256Literal(uint256 value) external {
        if (value == 2) {
            revert BrokenInvariantError({id: "GATED-UINT256", description: "value == 2"});
        }
    }

    function gatedByUint128Literal(uint128 value) external {
        if (value == 12345) {
            revert BrokenInvariantError({id: "GATED-UINT128", description: "value == 12345"});
        }
    }

    function gatedByInt256Literal(int256 value) external {
        if (value == -7) {
            revert BrokenInvariantError({id: "GATED-INT256", description: "value == -7"});
        }
    }

    function gatedByInt8Literal(int8 value) external {
        if (value == -3) {
            revert BrokenInvariantError({id: "GATED-INT8", description: "value == -3"});
        }
    }

    function gatedByBytes32Literal(bytes32 hash) external {
        if (hash == bytes32(0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef)) {
            revert BrokenInvariantError({id: "GATED-BYTES32", description: "hash == 0x123456..."});
        }
    }

    function gatedByBytes1Literal(bytes1 tag) external {
        if (tag == bytes1(0xab)) {
            revert BrokenInvariantError({id: "GATED-BYTES1", description: "tag == 0xab"});
        }
    }

    function gatedByAddressLiteral(address account) external {
        if (account == 0x5B38Da6a701c568545dCfcB03FcB875f56beddC4) {
            revert BrokenInvariantError({id: "GATED-ADDRESS", description: "account == 0x5B38..."});
        }
    }

    function gatedByBytesLiteral(bytes memory data) external {
        if (keccak256(data) == keccak256(hex"deadbeef")) {
            revert BrokenInvariantError({id: "GATED-BYTES", description: "keccak256(data) == keccak256(0xdeadbeef)"});
        }
    }

    function gatedByStringLiteral(string memory text) external {
        if (keccak256(bytes(text)) == keccak256(bytes("gold"))) {
            revert BrokenInvariantError({id: "GATED-STRING", description: "text == gold"});
        }
    }

    function gatedByEtherLiteral(uint256 value) external {
        if (value == 1 ether) {
            revert BrokenInvariantError({id: "GATED-ETHER", description: "value == 1 ether"});
        }
    }
}
