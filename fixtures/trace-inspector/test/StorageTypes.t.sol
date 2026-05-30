// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {StorageTypesRevert} from "../src/StorageTypes.sol";

contract StorageTypesTest {
    function testDeploy() external {
        new StorageTypesRevert();
    }
}
