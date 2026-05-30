// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./PrankVictim.sol";

/// @notice Minimal stateful-fuzzing target for raptor prank cheatcodes.
///
/// Setup establishes a persistent `vm.startPrank(ADMIN)` so that every
/// action during `chain.exec` sees ADMIN as `msg.sender` unless it
/// explicitly changes or stops the prank.  Invariants verify the
/// expected sender for each scenario.
contract PrankTarget {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    PrankVictim public victim;

    address[] public actors;
    address public currentActor;

    address constant ADMIN = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;
    address constant USER = 0xCafEBAbECAFEbAbEcaFEbabECAfebAbEcAFEBaBe;
    address constant ALICE = 0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa;

    address public lastSender;

    modifier useActor(uint256 actorSeed) {
        currentActor = actors[actorSeed % actors.length];
        vm.startPrank(currentActor);
        _;
        vm.stopPrank();
    }

    function setup() external {
        victim = new PrankVictim();
        actors = [
            address(0x1000000000000000000000000000000000000001),
            address(0x2000000000000000000000000000000000000002),
            address(0x3000000000000000000000000000000000000003)
        ];
        vm.startPrank(ADMIN);
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Call the victim without any prank cheatcode.  The persistent
    /// startPrank set during setup should still apply.
    function actionNestedCall() external {
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Overwrite the used startPrank with a different user.
    function actionOverwriteStart() external {
        vm.startPrank(USER);
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Stop the persistent prank so the real caller (this contract)
    /// becomes `msg.sender`.
    function actionStopPrank() external {
        vm.stopPrank();
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Stop the current prank and restore the canonical admin prank.
    function actionRestoreAdmin() external {
        vm.stopPrank();
        vm.startPrank(ADMIN);
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Use the `useActor` modifier to prank as a chosen actor.
    function actionUseActor(uint256 actorSeed) external useActor(actorSeed) {
        victim.record();
        lastSender = victim.lastSender();
    }

    /// Read the stored last sender.
    function getLastSender() external view returns (address) {
        return lastSender;
    }

    /// vm.prank twice without consuming the first must revert.
    function actionRevertDoublePrank() external {
        vm.prank(ALICE);
        vm.prank(USER);
    }

    /// vm.startPrank twice without using the first must revert.
    function actionRevertDoubleStart() external {
        vm.startPrank(ALICE);
        vm.startPrank(USER);
    }

    /// vm.prank over an active startPrank must revert.
    function actionRevertPrankOverStart() external {
        vm.startPrank(ALICE);
        vm.prank(USER);
    }

    /// Invariant: lastSender must be the admin address.
    function invariant_senderIsAdmin() external view {
        assert(lastSender == ADMIN);
    }

    /// Invariant: lastSender must be the user address.
    function invariant_senderIsUser() external view {
        assert(lastSender == USER);
    }

    /// Invariant: lastSender must be this contract (no prank active).
    function invariant_senderIsTarget() external view {
        assert(lastSender == address(this));
    }

    /// Invariant: lastSender is always one of the known valid addresses.
    function invariant_senderValid() external view {
        assert(
            lastSender == ADMIN ||
            lastSender == USER ||
            lastSender == address(this) ||
            lastSender == actors[0] ||
            lastSender == actors[1] ||
            lastSender == actors[2]
        );
    }
}
