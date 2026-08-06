// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library CoverageDocumentation {
    /// @dev Used as a prefix to some data.
    /// @dev Explanation of the prefix:
    /// hex       opcode          stack              comments
    /// ------------------------------------------------------------------------------
    /// 60 0b     PUSH1 0x0b      [11]               11 = length(prefix)
    /// 38        CODESIZE        [codesize, 11]
    /// 03        SUB             [len]              with len = codesize - 11
    /// 80        DUP1            [len, len]
    /// 60 0b     PUSH1 0x0b      [11, len, len]     code offset = 11
    /// 5f        PUSH0           [0, 11, len, len]  mem offset = 0
    /// 39        CODECOPY        [len]              mem[0:len] <- code[11:11+len]
    /// 5f        PUSH0           [0, len]           return offset = 0
    /// f3        RETURN          []                 mem[0:len] is returned
    bytes constant PREFIX = hex"600b380380600b5f395ff3";

    function getPrefixHash(bytes32 salt) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(PREFIX, salt));
    }

    function getPrefixHashTwice(bytes32 salt1, bytes32 salt2) internal pure returns (bytes32, bytes32) {
        return (keccak256(abi.encodePacked(PREFIX, salt1)), keccak256(abi.encodePacked(PREFIX, salt2)));
    }
}
