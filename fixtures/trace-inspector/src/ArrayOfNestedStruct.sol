// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Regression fixture for a trace-inspector edge case where storage
/// changes to a dynamic array of structs that contain nested struct fields
/// are not decoded correctly. The nested struct's sub-fields beyond the first
/// slot must be resolved to human-readable labels (e.g. `entries[0].data.b`
/// instead of a raw keccak hash).
///
/// Also exercises dynamic arrays nested inside structs inside top-level
/// arrays (e.g. `entries[0].items[0].x`), where the data area is at
/// keccak256(length_slot) and must be resolved through recorded KECCAK256
/// results.
contract ArrayOfNestedStruct {
    struct Inner {
        uint256 a;
        uint256 b;
        uint256 c;
    }

    struct Item {
        uint256 x;
    }

    struct Entry {
        Inner data;
        bytes extra;
        Item[] items;
    }

    Entry[] public entries;

    constructor() {
        // Push one entry with nested struct fields set plus nested dynamic
        // array storage.
        entries.push();
        entries[0].data.a = 1;
        entries[0].data.b = 2;
        entries[0].data.c = 3;
        entries[0].extra = hex"cafebabe";
        entries[0].items.push(Item({x: 42}));
        revert("nested struct in array");
    }
}
