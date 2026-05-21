// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeLabel {
    Vm constant vm = Vm(address(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1));

    address public constant TARGET = address(0xBEEF);
    address public constant OTHER  = address(0xCAFE);

    // --- setup interaction ---

    function setup() external {
        vm.label(TARGET, "TargetFromSetup");
    }

    function setup_label_persists() external view returns (bool) {
        return keccak256(bytes(vm.getLabel(TARGET))) == keccak256(bytes("TargetFromSetup"));
    }

    function setup_only() external view returns (bool) {
        return bytes(vm.getLabel(OTHER)).length == 0;
    }

    // --- Same-sequence persistence ---

    function call_label(address addr, string calldata name) external {
        vm.label(addr, name);
    }

    function call_getLabel(address addr) external view returns (string memory) {
        return vm.getLabel(addr);
    }

    function label_persists_across_calls() external view returns (bool) {
        return keccak256(bytes(vm.getLabel(OTHER))) == keccak256(bytes("OtherLabel"));
    }

    // --- Overwrite ---

    function call_label_twice() external {
        vm.label(TARGET, "First");
        vm.label(TARGET, "Second");
    }

    function overwrite() external view returns (bool) {
        return keccak256(bytes(vm.getLabel(TARGET))) == keccak256(bytes("Second"));
    }

    // --- Revert safety ---

    function call_label_then_revert() external {
        vm.label(TARGET, "RevertedLabel");
        revert("intentional");
    }

    function revert_does_not_undo_label() external view returns (bool) {
        // label is metadata, not state; it survives the revert
        return keccak256(bytes(vm.getLabel(TARGET))) == keccak256(bytes("RevertedLabel"));
    }

    // --- Edge: empty string ---

    function call_label_empty() external {
        vm.label(TARGET, "");
    }

    function empty_label() external view returns (bool) {
        return bytes(vm.getLabel(TARGET)).length == 0;
    }

    // --- Edge: address(0) ---

    function call_label_zero() external {
        vm.label(address(0), "ZeroAddress");
    }

    function label_zero() external view returns (bool) {
        return keccak256(bytes(vm.getLabel(address(0)))) == keccak256(bytes("ZeroAddress"));
    }

    // --- setup + sequence label interaction ---

    function call_label_overrides_setup() external {
        vm.label(TARGET, "Overridden");
    }

    function setup_override() external view returns (bool) {
        return keccak256(bytes(vm.getLabel(TARGET))) == keccak256(bytes("Overridden"));
    }
}
