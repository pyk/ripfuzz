// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAssert {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    bool public passed;

    function setUp() external {
        passed = true;
    }

    function call_assert_true() external {
        vm.assertTrue(true);
    }

    function call_assert_eq_uint() external {
        vm.assertEq(uint256(42), uint256(42));
    }

    function call_assert_lt() external {
        vm.assertLt(uint256(1), uint256(2));
    }

    function property_assertions_ok() external view returns (bool) {
        return passed;
    }
}
