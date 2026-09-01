// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

interface ICounter {
    function value() external view returns (uint256);

    function increment(uint256 amount) external;
}
