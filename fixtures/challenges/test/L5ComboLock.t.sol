// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {ComboLock} from "../src/L5ComboLock.sol";

contract L5ComboLockTest {
    ComboLock public lock;

    function setUp() public {
        lock = new ComboLock();
    }

    function testCatchDragon() public {
        assert(!lock.property_caught());
        lock.prime(17);
        lock.even(42);
        lock.odd(99);
        assert(lock.property_caught());
    }
}
