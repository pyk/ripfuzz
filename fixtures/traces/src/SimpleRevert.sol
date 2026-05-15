// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract SimpleRevert {
    constructor() {
        revert("simple revert reason");
    }
}
