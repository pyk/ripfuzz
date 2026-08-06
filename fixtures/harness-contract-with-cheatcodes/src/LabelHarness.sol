// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract LabelHarness {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address constant ADMIN = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    address constant USER = 0xCafEBAbECAFEbAbEcaFEbabECAfebAbEcAFEBaBe;

    string constant ADMIN_LABEL = "admin";
    string constant USER_LABEL = "user";

    string public adminLabel;
    string public userLabel;

    function setup() external {
        vm.label(ADMIN, ADMIN_LABEL);
        vm.label(USER, USER_LABEL);
        adminLabel = vm.getLabel(ADMIN);
        userLabel = vm.getLabel(USER);
    }

    /// Relabel admin to a non-canonical value and store it.
    function actionRelabelAdmin() external {
        vm.label(ADMIN, "attacker");
        adminLabel = vm.getLabel(ADMIN);
    }

    /// Restore canonical labels for both addresses.
    function actionRestoreLabels() external {
        vm.label(ADMIN, ADMIN_LABEL);
        vm.label(USER, USER_LABEL);
        adminLabel = vm.getLabel(ADMIN);
        userLabel = vm.getLabel(USER);
    }

    /// Overwrite admin label multiple times, ending on the canonical value.
    function actionOverwriteAdmin() external {
        vm.label(ADMIN, "temp1");
        vm.label(ADMIN, "temp2");
        vm.label(ADMIN, ADMIN_LABEL);
        adminLabel = vm.getLabel(ADMIN);
    }

    /// Relabel user to a non-canonical value and store it.
    function actionRelabelUser() external {
        vm.label(USER, "hacker");
        userLabel = vm.getLabel(USER);
    }

    /// Restore only the user label.
    function actionRestoreUser() external {
        vm.label(USER, USER_LABEL);
        userLabel = vm.getLabel(USER);
    }

    /// Read the admin label directly from the cheatcode inspector.
    /// Used to prove that vm.label set in setup persists into exec.
    function getAdminLabelDirect() external view returns (string memory) {
        return vm.getLabel(ADMIN);
    }

    /// Invariant: both stored labels must match their canonical values.
    function invariant_labelsMatch() external view {
        assert(keccak256(bytes(adminLabel)) == keccak256(bytes(ADMIN_LABEL)));
        assert(keccak256(bytes(userLabel)) == keccak256(bytes(USER_LABEL)));
    }
}
