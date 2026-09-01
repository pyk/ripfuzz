// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract DepthHelper {
    function doSomething() external {}
}

contract CoverageDepth {
    DepthHelper public helper;

    function setup() external {
        helper = new DepthHelper();
    }

    function callDirect() external {
        helper.doSomething();
    }

    function callIndirect() external {
        this.callDirect();
    }
}
