// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: easy
///
/// Every handler asserts `false` behind a comparison with a literal. The
/// fuzzer must extract the literals from this source and use them as
/// arguments to reach every gate. One gate per literal kind:
/// `bool`, `uint256`, `uint128`, `int256`, `int8`, `bytes32`, `bytes1`,
/// `address`, `bytes`, `string`, and the `1 ether` subdenomination.
contract GatedByLiterals {
    function gatedByBoolLiteral(bool flag) external pure {
        if (flag == true) {
            assert(false);
        }
    }

    function gatedByUint256Literal(uint256 value) external pure {
        if (value == 2) {
            assert(false);
        }
    }

    function gatedByUint128Literal(uint128 value) external pure {
        if (value == 12345) {
            assert(false);
        }
    }

    function gatedByInt256Literal(int256 value) external pure {
        if (value == -7) {
            assert(false);
        }
    }

    function gatedByInt8Literal(int8 value) external pure {
        if (value == -3) {
            assert(false);
        }
    }

    function gatedByBytes32Literal(bytes32 hash) external pure {
        if (hash == bytes32(0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef)) {
            assert(false);
        }
    }

    function gatedByBytes1Literal(bytes1 tag) external pure {
        if (tag == bytes1(0xab)) {
            assert(false);
        }
    }

    function gatedByAddressLiteral(address account) external pure {
        if (account == 0x5B38Da6a701c568545dCfcB03FcB875f56beddC4) {
            assert(false);
        }
    }

    function gatedByBytesLiteral(bytes memory data) external pure {
        if (keccak256(data) == keccak256(hex"deadbeef")) {
            assert(false);
        }
    }

    function gatedByStringLiteral(string memory text) external pure {
        if (keccak256(bytes(text)) == keccak256(bytes("gold"))) {
            assert(false);
        }
    }

    function gatedByEtherLiteral(uint256 value) external pure {
        if (value == 1 ether) {
            assert(false);
        }
    }
}
