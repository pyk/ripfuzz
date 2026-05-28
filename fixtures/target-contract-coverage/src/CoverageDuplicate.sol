// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract DuplicateChild {
    function doSomething() external {}
}

contract CoverageDuplicate {
    DuplicateChild public child1;
    DuplicateChild public child2;

    function setup() external {
        child1 = new DuplicateChild();
        child2 = new DuplicateChild();
    }

    function callChild1() external {
        child1.doSomething();
    }

    function callChild2() external {
        child2.doSomething();
    }
}
