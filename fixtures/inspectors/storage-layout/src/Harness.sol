// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Base.sol";

contract Harness is Base {
    address public owner;

    uint128 public cap;

    uint128 public floor;

    uint256[] public amounts;
}
