// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

/// @title CoinbaseHandler
/// @notice Real-world fuzz handler that controls `block.coinbase` via the
///         `vm.coinbase` cheatcode.  Setup establishes a canonical coinbase and
///         actions mutate or restore it.  Invariants verify deterministic control.
contract CoinbaseHandler {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address constant EXPECTED_COINBASE = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    function setup() external {
        vm.coinbase(EXPECTED_COINBASE);
    }

    /// Invariant: the live coinbase must always match the expected address.
    function invariant_coinbase() external view {
        assert(block.coinbase == EXPECTED_COINBASE);
    }

    /// Action: re-set the coinbase to the expected address.
    function actionRestoreCoinbase() external {
        vm.coinbase(EXPECTED_COINBASE);
    }

    /// Action: temporarily set a different coinbase address.
    function actionMutateCoinbase() external {
        vm.coinbase(address(0x1111111111111111111111111111111111111111));
    }

    /// Action: interleave coinbase changes inside one tx, ending on expected.
    function actionCoinbaseSequence() external {
        vm.coinbase(address(0x1111111111111111111111111111111111111111));
        vm.coinbase(EXPECTED_COINBASE);
        vm.coinbase(address(0x2222222222222222222222222222222222222222));
        vm.coinbase(EXPECTED_COINBASE);
    }
}
