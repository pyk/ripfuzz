// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/PayableTarget.sol";

contract PayableCallTest {
    function testPayableCall() public payable {
        PayableTarget target = new PayableTarget();
        target.deposit{value: 1000}();
    }
}
