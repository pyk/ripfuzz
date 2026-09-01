// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

contract DeployChild {
    function doSomething() external {}
}

contract CoverageDeploy {
    function deployChild() external {
        DeployChild child = new DeployChild();
        child.doSomething();
    }
}
