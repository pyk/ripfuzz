// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: easy
///
/// The same as `Accumulate` but inherits `NoiseBase`, so the fuzzer must
/// reach the same highest value while 20 external functions revert or
/// mutate state that does not affect the value.
///
/// The highest value is `type(uint256).max`, reached by depositing
/// `type(uint256).max` while the total is still zero.
contract AccumulateWithNoise is NoiseBase {
    uint256 internal total;

    function deposit(uint256 amount) external {
        total += amount;
    }

    function value() external view returns (uint256) {
        return total;
    }
}
