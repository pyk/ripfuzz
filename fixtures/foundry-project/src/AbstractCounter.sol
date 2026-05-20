// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

abstract contract AbstractCounter {
    uint256 public count;

    function increment() external virtual;
}
