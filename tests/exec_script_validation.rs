//! `Script` validation against compiled fixtures: valid scripts convert
//! cleanly, invalid scripts fail with exact error messages.

use ripfuzz::exec::Script;
use ripfuzz::solc::{Solc, SolcOutput};

const VERSION: &str = "0.8.36";
const DIR: &str = "fixtures/exec-script-validation";

/// Compile a fixture script through solc into a temporary out directory.
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

/// A valid script must convert with the resolved `exec` function.
#[test]
fn accepts_valid_script() {
    let solc_output = solc_output("ScriptValid");

    let script = Script::try_from(&solc_output).unwrap();
    assert_eq!(script.id().name, "ScriptValid");
    assert_eq!(script.exec().name, "exec");
    assert_eq!(script.setup(), None);
}

/// A script with a `setup` function must capture it.
#[test]
fn accepts_setup_and_exec() {
    let solc_output = solc_output("ScriptWithSetup");

    let script = Script::try_from(&solc_output).unwrap();
    assert_eq!(script.setup().map(|f| f.name.as_str()), Some("setup"));
    assert_eq!(script.exec().name, "exec");
}

/// A script without an `exec` function must fail with a clear error.
#[test]
fn rejects_missing_exec() {
    let solc_output = solc_output("ScriptWithoutExec");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithoutExec.sol:ScriptWithoutExec` must define an `exec` function"
    );
}

/// An `exec` function with arguments must be rejected.
#[test]
fn rejects_exec_with_arguments() {
    let solc_output = solc_output("ScriptWithExecArgs");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithExecArgs.sol:ScriptWithExecArgs` function `exec` must take no arguments"
    );
}

/// A payable `exec` function must be rejected.
#[test]
fn rejects_payable_exec() {
    let solc_output = solc_output("ScriptWithPayableExec");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithPayableExec.sol:ScriptWithPayableExec` function `exec` must be `external` or `public`, found `payable`"
    );
}

/// A `setup` function with arguments must be rejected.
#[test]
fn rejects_setup_with_arguments() {
    let solc_output = solc_output("ScriptWithSetupArgs");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithSetupArgs.sol:ScriptWithSetupArgs` function `setup` must take no arguments"
    );
}

/// A payable `setup` function must be rejected.
#[test]
fn rejects_payable_setup() {
    let solc_output = solc_output("ScriptWithPayableSetup");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithPayableSetup.sol:ScriptWithPayableSetup` function `setup` must be `external` or `public`, found `payable`"
    );
}

/// A constructor with arguments must be rejected.
#[test]
fn rejects_constructor_with_arguments() {
    let solc_output = solc_output("ScriptWithConstructorArgs");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithConstructorArgs.sol:ScriptWithConstructorArgs` constructor must take no arguments"
    );
}

/// A payable constructor must be rejected.
#[test]
fn rejects_payable_constructor() {
    let solc_output = solc_output("ScriptWithPayableConstructor");

    let err = Script::try_from(&solc_output).unwrap_err();
    assert_eq!(
        err.to_string(),
        "script contract `fixtures/exec-script-validation/ScriptWithPayableConstructor.sol:ScriptWithPayableConstructor` constructor must not be `payable`"
    );
}
