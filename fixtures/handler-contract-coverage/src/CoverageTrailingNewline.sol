// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Regression fixture for a coverage reporter bug where a source map
/// entry whose offset points to the end of a newline-terminated file must not
/// produce a line number beyond the file's actual line count.
contract CoverageTrailingNewline {
    function foo() external pure returns (uint256) {
        return 1;
    }
}
