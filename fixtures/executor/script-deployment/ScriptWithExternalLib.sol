// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {ExternalMathLib} from "./ExternalMathLib.sol";

contract ScriptWithExternalLib {
    event ExecRan(uint256 total);

    uint256 public total;

    function exec() external {
        total = ExternalMathLib.add(total, 1);
        emit ExecRan(total);
    }
}
