// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract SummaryFail {
    bool public armed;

    function arm() external {
        armed = true;
    }

    function invariant_never_armed() external view {
        assert(!armed);
    }

    event Summarized(bool armed);

    function summary() external {
        emit Summarized(armed);
    }
}
