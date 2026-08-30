//! `MaxHarness` validation against compiled fixtures: valid harnesses convert
//! cleanly, invalid harnesses fail with exact error messages.

use revm::primitives::Bytes;

use ripfuzz::max::MaxHarness;
use ripfuzz::solc::{Solc, SolcOutput};
use ripfuzz::{DeployInput, TraceContext};

const VERSION: &str = "0.8.36";
const DIR: &str = "fixtures/max-harness-validation";

/// Compile a fixture harness through solc into a temporary out directory.
fn solc_output(name: &str) -> SolcOutput {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    Solc::new()
        .with_version(VERSION)
        .with_target(format!("{DIR}/{name}.sol"))
        .with_out(&out)
        .compile()
        .unwrap_or_else(|err| panic!("fixture {name} must compile: {err}"))
}

/// A valid harness must convert with the resolved `value` function.
#[test]
fn accepts_valid_harness() {
    let solc_output = solc_output("HarnessWithValue");

    let max_harness = MaxHarness::try_from(&solc_output).unwrap();
    assert_eq!(max_harness.id().name, "HarnessWithValue");
    assert_eq!(max_harness.value().name, "value");
    assert_eq!(max_harness.setup(), None);
    assert_eq!(max_harness.summary(), None);
}

/// A harness with `setup` and `summary` functions must capture both.
#[test]
fn accepts_setup_and_summary() {
    let solc_output = solc_output("HarnessWithSetupAndSummary");

    let max_harness = MaxHarness::try_from(&solc_output).unwrap();
    assert_eq!(max_harness.setup().map(|f| f.name.as_str()), Some("setup"));
    assert_eq!(
        max_harness.summary().map(|f| f.name.as_str()),
        Some("summary")
    );
}

/// A harness without a `value` function must fail with a clear error.
#[test]
fn rejects_missing_value() {
    let solc_output = solc_output("HarnessWithoutValue");

    let err = MaxHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithoutValue.sol:HarnessWithoutValue` must define a `value` function"
    );
}

/// A nonpayable `value` function must be rejected.
#[test]
fn rejects_non_view_value() {
    let solc_output = solc_output("HarnessWithNonViewValue");

    let err = MaxHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithNonViewValue.sol:HarnessWithNonViewValue` function `value` must be `view` or `pure`, found `nonpayable`"
    );
}

/// A `value` function returning a non-`uint256` type must be rejected.
#[test]
fn rejects_wrong_value_return() {
    let solc_output = solc_output("HarnessWithWrongValueReturn");

    let err = MaxHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithWrongValueReturn.sol:HarnessWithWrongValueReturn` function `value` must return `uint256`, found `uint128`"
    );
}

/// A harness declaring `invariant_*` functions must be rejected.
#[test]
fn rejects_invariant_functions() {
    let solc_output = solc_output("HarnessWithInvariantFunction");
    let err = MaxHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "max harness `fixtures/max-harness-validation/HarnessWithInvariantFunction.sol:HarnessWithInvariantFunction` must not define `invariant_*` functions, found: invariant_value_is_zero"
    );
}

/// The trace context built from a solc output must resolve the harness by its
/// initcode, so trace labels get the harness contract name for free.
#[test]
fn trace_context_resolves_harness_by_initcode() {
    let solc_output = solc_output("HarnessWithValue");

    let max_harness = MaxHarness::try_from(&solc_output).unwrap();
    let deploy_input = DeployInput::from(&max_harness);
    let hexcode = deploy_input
        .initcode
        .strip_prefix("0x")
        .unwrap_or(&deploy_input.initcode);
    let initcode = hex::decode(hexcode).unwrap();
    let ctx = TraceContext::from(&solc_output);

    assert_eq!(
        ctx.resolve_by_initcode(&Bytes::from(initcode)),
        Some("HarnessWithValue")
    );
}
