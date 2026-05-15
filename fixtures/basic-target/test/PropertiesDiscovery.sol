// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

contract PropertiesDiscovery {
    bool public flag;
    uint256 public count;

    // Valid property: view + bool + prefix.
    function property_flag_is_true() external view returns (bool) {
        return flag;
    }

    // Invalid: not view/pure.
    function property_not_view() external returns (bool) {
        count++;
        return true;
    }

    // Invalid: returns uint256 instead of bool.
    function property_returns_uint() external view returns (uint256) {
        return count;
    }

    // Invalid: no property_ prefix.
    function plain_function() external view returns (bool) {
        return flag;
    }
}
