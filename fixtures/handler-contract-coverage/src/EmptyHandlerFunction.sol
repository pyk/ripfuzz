// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RipFuzz} from "./RipFuzz.sol";

contract EmptyHandlerFunction is RipFuzz {
    function dummyHandlerFunction() external {}
}
