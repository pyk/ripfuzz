//! `MaxHarness` validation against compiled fixtures: valid harnesses convert
//! cleanly, invalid harnesses fail with exact error messages.

use ripfuzz::harness::Harness;
use ripfuzz::max::MaxHarness;
use ripfuzz::solc::Solc;

const VERSION: &str = "0.8.36";
const DIR: &str = "fixtures/max-harness-validation";

/// Compile a fixture harness through solc into a temporary out directory.
fn harness(name: &str) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    Solc::new()
        .with_version(VERSION)
        .with_target(format!("{DIR}/{name}.sol"))
        .with_out(&out)
        .compile()
        .unwrap_or_else(|err| panic!("fixture {name} must compile: {err}"))
}

/// A valid harness must convert and expose the resolved `value` function.
#[test]
fn accepts_valid_harness() {
    let max_harness = MaxHarness::try_from(harness("HarnessWithValue")).unwrap();

    assert_eq!(max_harness.id().name, "HarnessWithValue");
    assert_eq!(max_harness.value().name, "value");
    assert_eq!(max_harness.setup(), None);
    assert_eq!(max_harness.summary(), None);
}

/// A harness with `setup` and `summary` functions must capture both.
#[test]
fn accepts_setup_and_summary() {
    let max_harness = MaxHarness::try_from(harness("HarnessWithSetupAndSummary")).unwrap();

    assert_eq!(max_harness.setup().map(|f| f.name.as_str()), Some("setup"));
    assert_eq!(
        max_harness.summary().map(|f| f.name.as_str()),
        Some("summary")
    );
}

/// A harness without a `value` function must fail with a clear error.
#[test]
fn rejects_missing_value() {
    let harness = harness("HarnessWithoutValue");

    let err = MaxHarness::try_from(harness).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithoutValue.sol:HarnessWithoutValue` must define a `value` function"
    );
}

/// A nonpayable `value` function must be rejected.
#[test]
fn rejects_non_view_value() {
    let harness = harness("HarnessWithNonViewValue");

    let err = MaxHarness::try_from(harness).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithNonViewValue.sol:HarnessWithNonViewValue` function `value` must be `view` or `pure`, found `nonpayable`"
    );
}

/// A `value` function returning a non-`uint256` type must be rejected.
#[test]
fn rejects_wrong_value_return() {
    let harness = harness("HarnessWithWrongValueReturn");

    let err = MaxHarness::try_from(harness).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithWrongValueReturn.sol:HarnessWithWrongValueReturn` function `value` must return `uint256`, found `uint128`"
    );
}

/// A harness declaring `invariant_*` functions must be rejected.
#[test]
fn rejects_invariant_functions() {
    let harness = harness("HarnessWithInvariantFunction");

    let err = MaxHarness::try_from(harness).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithInvariantFunction.sol:HarnessWithInvariantFunction` must not define `invariant_*` functions, found: invariant_value_is_zero"
    );
}
