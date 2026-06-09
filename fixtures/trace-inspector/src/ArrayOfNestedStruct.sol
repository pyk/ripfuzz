// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Regression fixture for a trace-inspector edge case where storage
/// changes to a dynamic array of structs that contain nested struct fields
/// are not decoded correctly. The nested struct's sub-fields beyond the first
/// slot must be resolved to human-readable labels (e.g. `entries[0].data.b`
/// instead of a raw keccak hash).
contract ArrayOfNestedStruct {
    struct Inner {
        uint256 a;
        uint256 b;
        uint256 c;
    }

    struct Entry {
        Inner data;
        bytes extra;
    }

    Entry[] public entries;

    constructor() {
        // Push one entry with three Inner fields set, plus extra bytes.
        entries.push();
        entries[0].data.a = 1;
        entries[0].data.b = 2;
        entries[0].data.c = 3;
        entries[0].extra = hex"cafebabe";
        revert("nested struct in array");
    }
}
