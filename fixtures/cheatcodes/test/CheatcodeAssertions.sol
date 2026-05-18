// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";

contract CheatcodeAssertions {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    bool public setUpPassed;
    uint256 public recordedUint;
    int256 public recordedInt;
    bool public recordedBool;

    // -------------------------------------------------
    // 1. setUp interaction
    // -------------------------------------------------

    function setUp() external {
        vm.ensure(true, "setUp ensure must pass");
        setUpPassed = true;
    }

    function property_setup_ensure_passed() external view returns (bool) {
        return setUpPassed;
    }

    // -------------------------------------------------
    // 2. Happy path (all types)
    // -------------------------------------------------

    function call_ensure_true() external {
        vm.ensure(true, "should be true");
    }

    function call_deny_false() external {
        vm.deny(false, "should be false");
    }

    function call_eq_uint() external {
        vm.eq(uint256(42), uint256(42), "uint mismatch");
    }

    function call_eq_int() external {
        vm.eq(int256(-1), int256(-1), "int mismatch");
    }

    function call_eq_bool() external {
        vm.eq(true, true, "bool mismatch");
    }

    function call_eq_address() external {
        vm.eq(address(this), address(this), "address mismatch");
    }

    function call_eq_bytes32() external {
        vm.eq(bytes32(uint256(1)), bytes32(uint256(1)), "bytes32 mismatch");
    }

    function call_eq_string() external {
        string memory s = "hello";
        vm.eq(s, s, "string mismatch");
    }

    function call_eq_bytes() external {
        bytes memory b = bytes("hello");
        vm.eq(b, b, "bytes mismatch");
    }

    function call_ne_uint() external {
        vm.ne(uint256(1), uint256(2), "should not be equal");
    }

    function call_ne_int() external {
        vm.ne(int256(1), int256(2), "should not be equal");
    }

    function call_lt_uint() external {
        vm.lt(uint256(1), uint256(2), "1 should be < 2");
    }

    function call_lt_int() external {
        vm.lt(int256(-2), int256(-1), "-2 should be < -1");
    }

    function call_lte_uint() external {
        vm.lte(uint256(2), uint256(2), "2 should be <= 2");
    }

    function call_gt_uint() external {
        vm.gt(uint256(2), uint256(1), "2 should be > 1");
    }

    function call_gte_int() external {
        vm.gte(int256(-1), int256(-1), "-1 should be >= -1");
    }

    // -------------------------------------------------
    // 3. Failure path (triggers revert)
    // -------------------------------------------------

    function call_ensure_false_should_revert() external {
        vm.ensure(false, "expected true but got false");
    }

    function call_deny_true_should_revert() external {
        vm.deny(true, "expected false but got true");
    }

    function call_eq_uint_fail() external {
        vm.eq(uint256(1), uint256(2), "1 != 2");
    }

    function call_ne_uint_fail() external {
        vm.ne(uint256(2), uint256(2), "2 == 2");
    }

    function call_lt_uint_fail() external {
        vm.lt(uint256(2), uint256(1), "2 < 1 is false");
    }

    function call_lte_uint_fail() external {
        vm.lte(uint256(2), uint256(1), "2 <= 1 is false");
    }

    function call_gt_uint_fail() external {
        vm.gt(uint256(1), uint256(2), "1 > 2 is false");
    }

    function call_gte_int_fail() external {
        vm.gte(int256(-2), int256(-1), "-2 >= -1 is false");
    }

    // -------------------------------------------------
    // 4. Same-sequence persistence / abortion
    // -------------------------------------------------

    function call_record_then_fail() external {
        recordedUint = 123;
        vm.eq(uint256(1), uint256(2), "this fails");
    }

    function call_read_recorded() external view returns (uint256) {
        return recordedUint;
    }

    function property_recorded_after_failure() external view returns (bool) {
        // If call_record_then_fail reverted, recordedUint must still be 0
        // because the transaction rolled back.
        return recordedUint == 0;
    }

    // -------------------------------------------------
    // 5. Cross-sequence isolation
    // -------------------------------------------------

    function call_set_recorded(uint256 v) external {
        recordedUint = v;
    }

    function property_cross_sequence_isolation() external view returns (bool) {
        // If a previous corpus item called call_set_recorded(999), this
        // property must still see 0 because every corpus item clones base state.
        return recordedUint == 0;
    }

    // -------------------------------------------------
    // 6. Edge cases
    // -------------------------------------------------

    function call_eq_zero() external {
        vm.eq(uint256(0), uint256(0), "zero != zero");
    }

    function call_eq_max() external {
        vm.eq(type(uint256).max, type(uint256).max, "max != max");
    }

    function call_eq_empty_string() external {
        string memory s = "";
        vm.eq(s, s, "empty string mismatch");
    }

    function call_eq_empty_bytes() external {
        bytes memory b = bytes("");
        vm.eq(b, b, "empty bytes mismatch");
    }

    function call_lt_uint_zero_vs_max() external {
        vm.lt(uint256(0), type(uint256).max, "0 < max");
    }

    function call_gt_int_min_vs_max() external {
        vm.gt(type(int256).max, type(int256).min, "max > min");
    }

    // -------------------------------------------------
    // 7. Property checks after mixed sequences
    // -------------------------------------------------

    function property_all_happy_calls_succeed() external view returns (bool) {
        // Used when the sequence contains only passing assertions.
        return true;
    }

    function property_no_side_effect_leak() external view returns (bool) {
        // Asserting should never mutate contract state.
        return recordedUint == 0 && recordedInt == 0 && !recordedBool;
    }
}

