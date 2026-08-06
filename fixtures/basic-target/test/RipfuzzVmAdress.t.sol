// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {console} from "forge-std/console.sol";

contract RipfuzzVmAddressTest {
    /// @dev Cheat code address.
    /// Calculated as `address(uint160(uint256(keccak256("hevm cheat code"))))`.
    address internal constant VM_ADDRESS = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;

    function testRipfuzzVmAddress() external pure {
        address ripfuzzVm = address(uint160(uint256(keccak256("hevm cheat code"))));
        require(ripfuzzVm == VM_ADDRESS, "Ripfuzz VM address must match hevm cheat code");
        console.log("Ripfuzz VM address:", ripfuzzVm);
    }
}
