//! `TestHarness` validation against compiled fixtures: valid harnesses convert
//! cleanly, invalid harnesses fail with exact error messages.

use revm::primitives::Bytes;

use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::tester::TestHarness;
use ripfuzz::{DeployInput, TraceContext};

const VERSION: &str = "0.8.36";
const DIR: &str = "fixtures/tester/harness-validation";

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

/// A valid harness must convert with its handler and invariant captured.
#[test]
fn accepts_valid_harness() {
    let solc_output = solc_output("HarnessValid");

    let test_harness = TestHarness::try_from(&solc_output).unwrap();
    assert_eq!(test_harness.id().name, "HarnessValid");
    assert_eq!(test_harness.setup(), None);
    assert_eq!(test_harness.summary(), None);
    assert!(
        test_harness.handlers().iter().any(|f| f.name == "deposit"),
        "the handler function must be captured"
    );
    assert_eq!(test_harness.invariants().len(), 1);
    assert_eq!(test_harness.invariants()[0].name, "invariant_total");
}

/// A harness with `setup` and `summary` functions must capture both.
#[test]
fn accepts_setup_and_summary() {
    let solc_output = solc_output("HarnessWithSetupAndSummary");

    let test_harness = TestHarness::try_from(&solc_output).unwrap();
    assert_eq!(test_harness.setup().map(|f| f.name.as_str()), Some("setup"));
    assert_eq!(
        test_harness.summary().map(|f| f.name.as_str()),
        Some("summary")
    );
}

/// A harness with a constructor taking arguments must be rejected.
#[test]
fn rejects_constructor_args() {
    let solc_output = solc_output("HarnessWithConstructorArgs");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithConstructorArgs.sol:HarnessWithConstructorArgs` constructor must not take arguments"
    );
}

/// A payable constructor must be rejected.
#[test]
fn rejects_payable_constructor() {
    let solc_output = solc_output("HarnessWithPayableConstructor");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithPayableConstructor.sol:HarnessWithPayableConstructor` constructor must not be `payable`"
    );
}

/// A `setup` function taking arguments must be rejected.
#[test]
fn rejects_setup_args() {
    let solc_output = solc_output("HarnessWithSetupArgs");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithSetupArgs.sol:HarnessWithSetupArgs` function `setup` must not take arguments"
    );
}

/// A payable `setup` function must be rejected.
#[test]
fn rejects_payable_setup() {
    let solc_output = solc_output("HarnessWithPayableSetup");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithPayableSetup.sol:HarnessWithPayableSetup` function `setup` must not be `payable`"
    );
}

/// A `summary` function taking arguments must be rejected.
#[test]
fn rejects_summary_args() {
    let solc_output = solc_output("HarnessWithSummaryArgs");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithSummaryArgs.sol:HarnessWithSummaryArgs` function `summary` must not take arguments"
    );
}

/// A payable `summary` function must be rejected.
#[test]
fn rejects_payable_summary() {
    let solc_output = solc_output("HarnessWithPayableSummary");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithPayableSummary.sol:HarnessWithPayableSummary` function `summary` must not be `payable`"
    );
}

/// An `invariant_*` function taking arguments must be rejected.
#[test]
fn rejects_invariant_args() {
    let solc_output = solc_output("HarnessWithInvariantArgs");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithInvariantArgs.sol:HarnessWithInvariantArgs` function `invariant_total` must not take arguments"
    );
}

/// A payable `invariant_*` function must be rejected.
#[test]
fn rejects_payable_invariant() {
    let solc_output = solc_output("HarnessWithPayableInvariant");

    let err = TestHarness::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test harness `fixtures/tester/harness-validation/HarnessWithPayableInvariant.sol:HarnessWithPayableInvariant` function `invariant_total` must not be `payable`"
    );
}

/// The trace context built from a solc output must resolve the harness by its
/// initcode, so trace labels get the harness contract name for free.
#[test]
fn trace_context_resolves_harness_by_initcode() {
    let solc_output = solc_output("HarnessValid");

    let test_harness = TestHarness::try_from(&solc_output).unwrap();
    let deploy_input = DeployInput::from(&test_harness);
    let hexcode = deploy_input
        .initcode
        .strip_prefix("0x")
        .unwrap_or(&deploy_input.initcode);
    let initcode = hex::decode(hexcode).unwrap();
    let ctx = TraceContext::from(&solc_output);

    assert_eq!(
        ctx.resolve_by_initcode(&Bytes::from(initcode)),
        Some("HarnessValid")
    );
}
