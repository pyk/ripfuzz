// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

contract InvalidSummaryView {
    function touch() external {}

    function summary() external view {}
}
