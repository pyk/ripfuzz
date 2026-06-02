// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract CoverageInitcodeChild {
    uint256 public x;
    constructor(uint256 _x) {
        x = _x;
    }
    function doSomething() external {}
}

contract CoverageInitcodeFactory {
    function createChild(uint256 x) external {
        CoverageInitcodeChild child = new CoverageInitcodeChild(x);
        child.doSomething();
    }
}
