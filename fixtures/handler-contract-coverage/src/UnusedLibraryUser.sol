// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {UnusedLibrary} from "./UnusedLibrary.sol";

contract UnusedLibraryUser {
    function useAdd(uint256 a, uint256 b) external pure returns (uint256) {
        return UnusedLibrary.usedAdd(a, b);
    }
}
