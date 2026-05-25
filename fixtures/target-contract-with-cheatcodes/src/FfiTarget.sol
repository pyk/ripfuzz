// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";

contract FfiTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 constant EXPECTED_VALUE = 42;

    uint256 public storedValue;

    function setup() external {
        storedValue = _ffiToUint42();
    }

    function getValue() external view returns (uint256) {
        return storedValue;
    }

    /// Call vm.ffi with the same args twice in one tx to prove determinism.
    function callFfiSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        first = _ffiToUint42();
        second = _ffiToUint42();
    }

    /// Call vm.ffi with different values and interleave to prove
    /// sequence independence.
    function callFfiSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        first = _ffiToUint1();
        second = _ffiToUint42();
        third = _ffiToUint5();
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callFfiAndWarp()
        external
        returns (uint256 value, uint256 timestamp)
    {
        value = _ffiToUint42();
        vm.warp(1234567890);
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-run ffi and store the result.
    function actionFfi() external {
        storedValue = _ffiToUint42();
    }

    function invariant_ffi() external view {
        assert(storedValue == EXPECTED_VALUE);
    }

    function _makeFfiArgs(
        string memory hexValue
    ) internal pure returns (string[] memory args) {
        args = new string[](3);
        args[0] = "printf";
        args[1] = "%s";
        args[2] = hexValue;
    }

    function _ffiToUint1() internal returns (uint256) {
        string[] memory args = _makeFfiArgs(
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
        return abi.decode(vm.ffi(args), (uint256));
    }

    function _ffiToUint42() internal returns (uint256) {
        string[] memory args = _makeFfiArgs(
            "000000000000000000000000000000000000000000000000000000000000002a"
        );
        return abi.decode(vm.ffi(args), (uint256));
    }

    function _ffiToUint5() internal returns (uint256) {
        string[] memory args = _makeFfiArgs(
            "0000000000000000000000000000000000000000000000000000000000000005"
        );
        return abi.decode(vm.ffi(args), (uint256));
    }

    function _ffiToUint100() internal returns (uint256) {
        string[] memory args = _makeFfiArgs(
            "0000000000000000000000000000000000000000000000000000000000000064"
        );
        return abi.decode(vm.ffi(args), (uint256));
    }
}
