// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

contract PropertiesDiscovery {
    bool public flag;
    uint256 public count;

    // Valid invariant: view + prefix.
    function invariant_flag_is_true() external view returns (bool) {
        return flag;
    }

    // Invalid: not view/pure.
    function invariant_not_view() external returns (bool) {
        count++;
        return true;
    }

    // Valid: returns uint256 (bool check removed).
    function invariant_returns_uint() external view returns (uint256) {
        return count;
    }

    // Invalid: no invariant_ prefix.
    function plain_function() external view returns (bool) {
        return flag;
    }
}
