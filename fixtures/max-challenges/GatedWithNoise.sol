// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: medium
///
/// The same as `Gated` but inherits `NoiseBase`, so the fuzzer must reach
/// the same highest value while 20 external functions revert or mutate
/// state that does not affect the value.
///
/// The highest value is `type(uint256).max`, reached by calling `enter`
/// before a deposit of `type(uint256).max`. Deposits before `enter` revert.
contract GatedWithNoise is NoiseBase {
    bool internal entered;
    uint256 internal total;

    function enter() external {
        entered = true;
    }

    function deposit(uint256 amount) external {
        require(entered);
        total += amount;
    }

    function value() external view returns (uint256) {
        return total;
    }
}
