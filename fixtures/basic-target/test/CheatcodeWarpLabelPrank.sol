// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeWarpLabelPrank {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public recordedTimestamp;
    address public recordedCaller;

    function setUp() external {
        vm.warp(1234567890);
        vm.label(address(this), "TargetContract");
    }

    function action() external {
        recordedTimestamp = block.timestamp;
        recordedCaller = msg.sender;
    }

    function property_timestamp_correct() external view returns (bool) {
        return recordedTimestamp == 1234567890;
    }
}
