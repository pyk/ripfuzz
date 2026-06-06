// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {LeafContract} from "./LeafContract.sol";

contract MiddleContract {
    LeafContract public leaf;

    function createLeaf() external {
        leaf = new LeafContract();
    }

    function invariant_leaf_exists() external view {
        assert(address(leaf) != address(0));
    }
}
