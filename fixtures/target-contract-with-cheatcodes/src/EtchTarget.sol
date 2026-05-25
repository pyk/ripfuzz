// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./Counter.sol";
import "./AltCounter.sol";

contract EtchTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    address constant ETCH_ADDR = 0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF;

    uint256 public storedValue;

    function setup() external {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        storedValue = Counter(ETCH_ADDR).getValue();
    }

    function getEtchedValue() external pure returns (uint256) {
        return Counter(ETCH_ADDR).getValue();
    }

    function getStoredValue() external view returns (uint256) {
        return storedValue;
    }

    /// Call vm.etch with the same code twice in one tx to prove
    /// the cheatcode is deterministic.
    function callEtchSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        first = Counter(ETCH_ADDR).getValue();
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        second = Counter(ETCH_ADDR).getValue();
    }

    /// Call vm.etch with different codes and interleave to prove
    /// sequence independence and code uniqueness.
    function callEtchSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        first = Counter(ETCH_ADDR).getValue();
        vm.etch(ETCH_ADDR, type(AltCounter).runtimeCode);
        second = AltCounter(ETCH_ADDR).getValue();
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        third = Counter(ETCH_ADDR).getValue();
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callEtchAndWarp()
        external
        returns (uint256 value, uint256 timestamp)
    {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        vm.warp(1234567890);
        value = Counter(ETCH_ADDR).getValue();
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-etch the expected code and store the result.
    function actionEtch() external {
        vm.etch(ETCH_ADDR, type(Counter).runtimeCode);
        storedValue = Counter(ETCH_ADDR).getValue();
    }

    function invariant_etch() external view {
        assert(storedValue == 42);
    }
}
