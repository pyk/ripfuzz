// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CoverageDocumentation} from "./CoverageDocumentation.sol";

/// @title CoverageDocumentationUser
/// @dev A contract that uses the CoverageDocumentation library.
contract CoverageDocumentationUser {
    function getPrefixHash(bytes32 salt) external pure returns (bytes32) {
        return CoverageDocumentation.getPrefixHash(salt);
    }
}
