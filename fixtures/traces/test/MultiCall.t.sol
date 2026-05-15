// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/MultiCall.sol";
import "../src/Helper.sol";

contract MultiCallTest {
    function testMultiCall() public {
        Helper helper = new Helper();
        MultiCall mc = new MultiCall();
        mc.doManyCalls(address(helper));
    }
}
