// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// @notice Regression fixture for a trace-inspector edge case where a fixed
/// array inside a struct inside a mapping is stored, but the slot is beyond
/// the struct field's base slot, so it is not decoded without special handling.
contract ArrayInStructMapping {
    struct Data {
        uint256 a;
        uint256[10] arr;
    }

    mapping(uint256 => Data) public data;

    constructor() {
        assembly {
            mstore(0x00, 1)
            mstore(0x20, 0)
            let base := keccak256(0x00, 0x40)
            // arr[1] is at base + 2 (a is at 0, arr starts at 1, arr[1] is at 2)
            sstore(add(base, 2), 42)
        }
        revert("array in struct mapping");
    }
}
