// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Regression fixture for a trace-inspector edge case where a struct
/// field inside a mapping is stored, but the exact mapping base slot never
/// appears in an SSTORE because the first field is never touched.
contract StructMappingSlot {
    struct Data {
        uint256 a;
        uint256 b;
        uint256 c;
    }

    mapping(uint256 => Data) public data;

    constructor() {
        // Store only the third field (c). The first two fields are never set,
        // so the exact mapping base slot never appears in an SSTORE.
        assembly {
            // Compute mapping slot for key = 1
            mstore(0x00, 1)
            mstore(0x20, 0)
            let base := keccak256(0x00, 0x40)
            // c is at slot base + 2
            sstore(add(base, 2), 42)
        }
        revert("struct mapping slot");
    }
}
