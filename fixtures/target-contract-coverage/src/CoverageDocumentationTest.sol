// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CoverageDocumentation} from "./CoverageDocumentation.sol";

/// @title CoverageDocumentationTest
/// @dev A test contract that imports the library but is never deployed.
contract CoverageDocumentationTest {
    function testPrefix(bytes32 salt) external pure returns (bytes32) {
        return CoverageDocumentation.getPrefixHash(salt);
    }

    function testPrefixTwice(bytes32 salt1, bytes32 salt2) external pure returns (bytes32, bytes32) {
        return CoverageDocumentation.getPrefixHashTwice(salt1, salt2);
    }
}
