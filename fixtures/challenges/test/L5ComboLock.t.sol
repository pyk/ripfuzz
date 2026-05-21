// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

import {ComboLock} from "../src/L5ComboLock.sol";

contract L5ComboLockTest {
    ComboLock public lock;

    function setup() public {
        lock = new ComboLock();
    }

    function testCatchDragon() public {
        lock.invariant_caught(); // succeeds before dragon
        lock.prime(17);
        lock.even(42);
        lock.odd(99);
        try lock.invariant_caught() {
            revert("invariant should have reverted after dragon");
        } catch {
            // expected revert — dragon caught!
        }
    }
}
