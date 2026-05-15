// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract LabelCallTrace {
    constructor() {
        address target = 0x1111111111111111111111111111111111111111;

        // setValue(42)
        (bool ok1, ) = target.call(abi.encodeWithSelector(0x55241077, 42));
        require(ok1, "setValue failed");

        // getValue() -> should return 42
        (bool ok2, bytes memory ret) = target.call(abi.encodeWithSelector(0x20965255));
        require(ok2, "getValue failed");
        uint256 v = abi.decode(ret, (uint256));
        require(v == 42, "value mismatch");
    }
}
