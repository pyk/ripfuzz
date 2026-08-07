// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {console} from "forge-std/console.sol";

contract RipfuzzRvmAddressTest {
    /// @dev Cheat code address.
    /// Calculated as `address(uint160(uint256(keccak256("ripfuzz cheatcode"))))`.
    address internal constant RVM_ADDRESS = 0x628dC59F11F72B611132eC40437F125ba1312F08;

    function testRipfuzzRvmAddress() external pure {
        address rvm = address(uint160(uint256(keccak256("ripfuzz cheatcode"))));
        require(rvm == RVM_ADDRESS, "Ripfuzz RVM address must match ripfuzz cheatcode");
        console.log("Ripfuzz RVM address:", rvm);
    }
}
