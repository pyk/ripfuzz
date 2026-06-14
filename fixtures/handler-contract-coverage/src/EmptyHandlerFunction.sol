// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RaptorFuzz} from "./RaptorFuzz.sol";

contract EmptyHandlerFunction is RaptorFuzz {
    function dummyHandlerFunction() external {}
}
