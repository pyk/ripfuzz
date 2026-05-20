// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

interface ICounter {
    function count() external view returns (uint256);
    function increment() external;
    function increment(uint256 amount) external;
}
