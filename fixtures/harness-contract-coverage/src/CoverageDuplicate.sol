// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract DuplicateChild {
    function doSomething() external {}
}

contract CoverageDuplicate {
    DuplicateChild public child1;
    DuplicateChild public child2;
    DuplicateChild[] public children;

    function setup() external {
        child1 = new DuplicateChild();
        child2 = new DuplicateChild();
        children.push(child1);
        children.push(child2);
    }

    function callChild1() external {
        child1.doSomething();
    }

    function callChild2() external {
        child2.doSomething();
    }

    function callChild(uint256 idx) external {
        children[idx % children.length].doSomething();
    }
}
