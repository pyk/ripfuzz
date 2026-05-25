// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract LabelTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant LABEL_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    string constant EXPECTED_LABEL = "DeadBeef";

    string public storedLabel;

    function setup() external {
        vm.label(LABEL_ADDR, EXPECTED_LABEL);
        storedLabel = vm.getLabel(LABEL_ADDR);
    }

    function getLabelFor(
        address addr
    ) external view returns (string memory label) {
        label = vm.getLabel(addr);
    }

    function getStoredLabel() external view returns (string memory label) {
        label = storedLabel;
    }

    /// Call vm.label with the same value twice in one tx to prove
    /// the cheatcode is deterministic.
    function callLabelSameValueTwice()
        external
        returns (string memory first, string memory second)
    {
        vm.label(address(this), "Self");
        first = vm.getLabel(address(this));
        second = vm.getLabel(address(this));
    }

    /// Call vm.label with different values and interleave to prove
    /// sequence independence and value uniqueness.
    function callLabelSequence()
        external
        returns (string memory first, string memory second, string memory third)
    {
        vm.label(address(this), "First");
        first = vm.getLabel(address(this));
        vm.label(address(this), "Second");
        second = vm.getLabel(address(this));
        vm.label(address(this), "First");
        third = vm.getLabel(address(this));
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callLabelAndWarp()
        external
        returns (string memory label, uint256 timestamp)
    {
        vm.label(address(this), "Labeled");
        vm.warp(1234567890);
        label = vm.getLabel(address(this));
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-label the expected address and store the result.
    function actionLabel() external {
        vm.label(LABEL_ADDR, EXPECTED_LABEL);
        storedLabel = vm.getLabel(LABEL_ADDR);
    }

    /// Edge case: getLabel on an unlabeled address must return empty string.
    function getUnlabeled(
        address addr
    ) external view returns (string memory label) {
        label = vm.getLabel(addr);
    }

    function invariant_label() external view {
        assert(
            keccak256(bytes(storedLabel)) == keccak256(bytes(EXPECTED_LABEL))
        );
    }
}
