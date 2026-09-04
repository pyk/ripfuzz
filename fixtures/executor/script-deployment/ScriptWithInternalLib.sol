// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {InternalMathLib} from "./InternalMathLib.sol";

contract ScriptWithInternalLib {
    event ExecRan(uint256 total);

    uint256 public total;

    function exec() external {
        total = InternalMathLib.add(total, 1);
        emit ExecRan(total);
    }
}
