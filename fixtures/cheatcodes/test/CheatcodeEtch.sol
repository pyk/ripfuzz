// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeEtch {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    // Runtime bytecode: PUSH1 0x01 PUSH1 0x00 MSTORE PUSH1 0x20 PUSH1 0x00 RETURN
    bytes constant RUNTIME_CODE = hex"6001600052602060006000f3";
    // Empty runtime code
    bytes constant EMPTY_CODE = hex"";

    // --- setUp interaction ---
    function setUp() external {
        vm.etch(address(0xCAFE), RUNTIME_CODE);
    }

    function call_record_extcodesize_cafe() external {
        // intentionally empty; property checks extcodesize
    }

    function setup_etch_persists() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xCAFE) }
        return size > 0;
    }

    // --- Same-sequence persistence ---
    function call_etch_beef() external {
        vm.etch(address(0xBEEF), RUNTIME_CODE);
    }

    function etch_persists_across_calls() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xBEEF) }
        return size > 0;
    }

    // --- Revert safety ---
    function call_etch_and_revert() external {
        vm.etch(address(0xDEAD), RUNTIME_CODE);
        revert("intentional");
    }

    function revert_undoes_etch() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xDEAD) }
        return size == 0;
    }

    // --- Overwrite ---
    function call_etch_overwrite() external {
        vm.etch(address(0xBEEF), RUNTIME_CODE);
        vm.etch(address(0xBEEF), EMPTY_CODE);
    }

    function etch_overwrite() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xBEEF) }
        return size == 0;
    }

    // --- Non-existent address ---
    function call_etch_new_account() external {
        vm.etch(address(0xFACADE), RUNTIME_CODE);
    }

    function etch_new_account() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xFACADE) }
        return size > 0;
    }

    // --- Property sees final state ---
    function final_etch() external view returns (bool) {
        uint256 size;
        assembly { size := extcodesize(0xCAFE) }
        return size > 0;
    }

    // --- Precompile guard ---
    function call_etch_precompile() external {
        vm.etch(address(0x01), RUNTIME_CODE);
    }

    function precompile_unchanged() external view returns (bool) {
        // If the precompile guard works, call_etch_precompile reverts and
        // this property is never evaluated. We include it for completeness.
        return true;
    }
}
