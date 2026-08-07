// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

contract LabelHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address constant ADMIN = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    address constant USER = 0xCafEBAbECAFEbAbEcaFEbabECAfebAbEcAFEBaBe;

    string constant ADMIN_LABEL = "admin";
    string constant USER_LABEL = "user";

    string public adminLabel;
    string public userLabel;

    function setup() external {
        rvm.label(ADMIN, ADMIN_LABEL);
        rvm.label(USER, USER_LABEL);
        adminLabel = rvm.getLabel(ADMIN);
        userLabel = rvm.getLabel(USER);
    }

    /// Relabel admin to a non-canonical value and store it.
    function actionRelabelAdmin() external {
        rvm.label(ADMIN, "attacker");
        adminLabel = rvm.getLabel(ADMIN);
    }

    /// Restore canonical labels for both addresses.
    function actionRestoreLabels() external {
        rvm.label(ADMIN, ADMIN_LABEL);
        rvm.label(USER, USER_LABEL);
        adminLabel = rvm.getLabel(ADMIN);
        userLabel = rvm.getLabel(USER);
    }

    /// Overwrite admin label multiple times, ending on the canonical value.
    function actionOverwriteAdmin() external {
        rvm.label(ADMIN, "temp1");
        rvm.label(ADMIN, "temp2");
        rvm.label(ADMIN, ADMIN_LABEL);
        adminLabel = rvm.getLabel(ADMIN);
    }

    /// Relabel user to a non-canonical value and store it.
    function actionRelabelUser() external {
        rvm.label(USER, "hacker");
        userLabel = rvm.getLabel(USER);
    }

    /// Restore only the user label.
    function actionRestoreUser() external {
        rvm.label(USER, USER_LABEL);
        userLabel = rvm.getLabel(USER);
    }

    /// Read the admin label directly from the cheatcode inspector.
    /// Used to prove that rvm.label set in setup persists into exec.
    function getAdminLabelDirect() external view returns (string memory) {
        return rvm.getLabel(ADMIN);
    }

    /// Invariant: both stored labels must match their canonical values.
    function invariant_labelsMatch() external view {
        assert(keccak256(bytes(adminLabel)) == keccak256(bytes(ADMIN_LABEL)));
        assert(keccak256(bytes(userLabel)) == keccak256(bytes(USER_LABEL)));
    }
}
