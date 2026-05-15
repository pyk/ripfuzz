// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/DelegateCallTarget.sol";

contract DelegateCallProxy {
    uint256 public value;

    function testDelegateCall() public {
        DelegateCallTarget target = new DelegateCallTarget();
        (bool ok, ) = address(target).delegatecall(
            abi.encodeWithSelector(DelegateCallTarget.setValue.selector, 99)
        );
        require(ok);
    }
}
