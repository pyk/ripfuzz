// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./RVM.sol";

/// @title CoinbaseHarness
/// @notice Real-world fuzz handler that controls `block.coinbase` via the
///         `rvm.coinbase` cheatcode.  Setup establishes a canonical coinbase and
///         actions mutate or restore it.  Invariants verify deterministic control.
contract CoinbaseHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    address constant EXPECTED_COINBASE = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    function setup() external {
        rvm.coinbase(EXPECTED_COINBASE);
    }

    /// Invariant: the live coinbase must always match the expected address.
    function invariant_coinbase() external view {
        assert(block.coinbase == EXPECTED_COINBASE);
    }

    /// Action: re-set the coinbase to the expected address.
    function actionRestoreCoinbase() external {
        rvm.coinbase(EXPECTED_COINBASE);
    }

    /// Action: temporarily set a different coinbase address.
    function actionMutateCoinbase() external {
        rvm.coinbase(address(0x1111111111111111111111111111111111111111));
    }

    /// Action: interleave coinbase changes inside one tx, ending on expected.
    function actionCoinbaseSequence() external {
        rvm.coinbase(address(0x1111111111111111111111111111111111111111));
        rvm.coinbase(EXPECTED_COINBASE);
        rvm.coinbase(address(0x2222222222222222222222222222222222222222));
        rvm.coinbase(EXPECTED_COINBASE);
    }
}
