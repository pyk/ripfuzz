// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {StorageTypesRevert} from "./StorageTypes.sol";

contract StorageTypesTest {
    function testDeploy() external {
        new StorageTypesRevert();
    }
}
