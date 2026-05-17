// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeWarp {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public recordedTimestamp;

    // --- setUp interaction ---

    function setUp() external {
        vm.warp(1234567890);
    }

    function action_record_timestamp() external {
        recordedTimestamp = block.timestamp;
    }

    function property_setup_warp_persists() external view returns (bool) {
        return recordedTimestamp == 1234567890;
    }

    // --- Same-sequence persistence ---

    function action_warp(uint256 ts) external {
        vm.warp(ts);
        recordedTimestamp = block.timestamp;
    }

    function property_warp_persists_across_calls() external view returns (bool) {
        // If action_warp(100) was call 0 and action_record_timestamp() was
        // call 1 with 0 delay, Medusa rules force +1 on the second call,
        // so we expect 101.
        return recordedTimestamp == 101;
    }

    // --- Revert safety ---

    function action_warp_and_revert(uint256 ts) external {
        vm.warp(ts);
        revert("intentional");
    }

    function property_revert_undoes_warp() external view returns (bool) {
        // After a reverted warp, the timestamp should NOT be the warped value.
        return block.timestamp != 9999;
    }

    // --- Delay interaction ---

    function action_warp_100() external {
        vm.warp(100);
    }

    function property_warp_with_delay() external view returns (bool) {
        // If action_warp_100() was call 0 and the next call had delay 5,
        // we expect 105.
        return block.timestamp == 105;
    }

    // --- Warp overwrite ---

    function action_warp_200() external {
        vm.warp(200);
    }

    function property_warp_overwrite() external view returns (bool) {
        // If action_warp_100() was call 0, action_warp_200() was call 1,
        // and this is call 2 with 0 delay, we expect 201 (200 + 1).
        return block.timestamp == 201;
    }
}
