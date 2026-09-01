// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: medium
///
/// The same as `Double` but inherits `NoiseBase`, so the fuzzer must reach
/// the same highest value while 20 external functions revert or mutate
/// state that does not affect the value.
///
/// The total starts at 1 and `double` repeats it. The highest value is
/// `2 ** max_calls`, reached by a sequence of only `double` calls. A single
/// `reset` wipes the total and any later doubles keep it at zero.
contract DoubleWithNoise is NoiseBase {
    uint256 internal total;

    constructor() {
        total = 1;
    }

    function double() external {
        total *= 2;
    }

    function reset() external {
        total = 0;
    }

    function value() external view returns (uint256) {
        return total;
    }
}
