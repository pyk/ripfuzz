// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import "./RVM.sol";

contract FfiHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    uint256 constant EXPECTED_VALUE = 42;

    uint256 public storedValue;

    function setup() external {
        storedValue = _ffiToUint42();
    }

    function getValue() external view returns (uint256) {
        return storedValue;
    }

    /// Re-run ffi and restore the canonical value.
    function actionFfi() external {
        storedValue = _ffiToUint42();
    }

    /// Mutate stored value to a different ffi-derived value.
    function actionMutateFfi() external {
        storedValue = _ffiToUint100();
    }

    /// Call rvm.ffi with the same args twice in one tx to prove determinism.
    function actionFfiSequence() external returns (uint256 first, uint256 second, uint256 third) {
        first = _ffiToUint1();
        second = _ffiToUint42();
        third = _ffiToUint5();
    }

    function invariant_ffi() external view {
        assert(storedValue == EXPECTED_VALUE);
    }

    function _makeFfiArgs(string memory hexValue) internal pure returns (string[] memory args) {
        args = new string[](3);
        args[0] = "printf";
        args[1] = "%s";
        args[2] = hexValue;
    }

    function _ffiToUint1() internal returns (uint256) {
        string[] memory args = _makeFfiArgs("0000000000000000000000000000000000000000000000000000000000000001");
        return abi.decode(rvm.ffi(args), (uint256));
    }

    function _ffiToUint42() internal returns (uint256) {
        string[] memory args = _makeFfiArgs("000000000000000000000000000000000000000000000000000000000000002a");
        return abi.decode(rvm.ffi(args), (uint256));
    }

    function _ffiToUint5() internal returns (uint256) {
        string[] memory args = _makeFfiArgs("0000000000000000000000000000000000000000000000000000000000000005");
        return abi.decode(rvm.ffi(args), (uint256));
    }

    function _ffiToUint100() internal returns (uint256) {
        string[] memory args = _makeFfiArgs("0000000000000000000000000000000000000000000000000000000000000064");
        return abi.decode(rvm.ffi(args), (uint256));
    }
}
