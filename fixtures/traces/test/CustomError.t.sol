// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "../src/CustomError.sol";

contract CustomErrorTest {
    function testCustomError() public {
        CustomError target = new CustomError();
        target.trigger();
    }
}
