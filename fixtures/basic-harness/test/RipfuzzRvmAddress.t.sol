// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {console} from "forge-std/console.sol";

contract RipfuzzRvmAddressTest {
    /// @dev Cheat code address.
    /// Calculated as `address(uint160(uint256(keccak256("hevm cheat code"))))`.
    address internal constant RVM_ADDRESS = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;

    function testRipfuzzRvmAddress() external pure {
        address rvm = address(uint160(uint256(keccak256("hevm cheat code"))));
        require(rvm == RVM_ADDRESS, "Ripfuzz RVM address must match hevm cheat code");
        console.log("Ripfuzz RVM address:", rvm);
    }
}
