// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// @notice Revert with this error to report a broken invariant to ripfuzz.
/// @dev The id deduplicates findings across the campaign, the description is
///      the human-readable reason shown in the output.
error BrokenInvariantError(string id, string description);
