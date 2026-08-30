// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: medium
///
/// The total starts at 1 and `double` repeats it. The highest value is
/// `2 ** max_calls`, reached by a sequence of only `double` calls. A single
/// `reset` wipes the total and any later doubles keep it at zero.
contract Double {
    uint256 internal total;

    constructor() {
        total = 1;
    }

    function double() external {
        unchecked {
            total *= 2;
        }
    }

    function reset() external {
        total = 0;
    }

    function value() external view returns (uint256) {
        return total;
    }
}
