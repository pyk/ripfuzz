//! `ripfuzz exec` compiles the script via solc, deploys it on a sandbox chain,
//! runs the optional `setup`, executes `exec`, and saves the execution trace.
//!
//! The fixtures under `fixtures/executor/script-deployment` are sources of the
//! project rooted at the current directory, and compilation artifacts are
//! shared under `./.ripfuzz/solc` namespaced by the script source path.

use std::fs;
use std::path::PathBuf;

use ripfuzz::cli::exec::Command;

const SCRIPT: &str = "fixtures/executor/script-deployment/ScriptExecutes.sol";
const REVERTING: &str = "fixtures/executor/script-deployment/ScriptWithRevertingConstructor.sol";
const REVERTING_SETUP: &str = "fixtures/executor/script-deployment/ScriptWithRevertingSetup.sol";
const REVERTING_EXEC: &str = "fixtures/executor/script-deployment/ScriptWithRevertingExec.sol";
const CUSTOM_NAME: &str = "fixtures/executor/script-deployment/ScriptCustomName.sol:CustomTarget";
const WRONG_NAME: &str = "fixtures/executor/script-deployment/ScriptCustomName.sol:DoesNotExist";
const MISSING_FILE: &str = "fixtures/executor/script-deployment/ScriptMissing.sol";

fn command(script: &str) -> Command {
    Command {
        script: script.parse().unwrap(),
        config: PathBuf::from("./ripfuzz.toml"),
        root: PathBuf::from("."),
        quiet: true,
        log_level: tracing::Level::INFO,
    }
}

/// A valid script must compile, deploy, run `setup`, and execute `exec`,
/// with the default contract name derived from the file stem.
#[test]
fn exec_compiles_and_runs_script() {
    command(SCRIPT)
        .run()
        .expect("exec should compile and run the script");

    assert!(
        traces_contain(".ripfuzz/traces", "ExecRan"),
        "a saved trace must contain the exec event"
    );
}

/// A script whose constructor reverts must fail deployment with a clear
/// error, and the execution trace must be dumped to the traces directory.
#[test]
fn exec_fails_when_constructor_reverts() {
    let err = command(REVERTING)
        .run()
        .expect_err("exec must fail when deployment reverts");
    assert_eq!(
        err.to_string(),
        "script contract `ScriptWithRevertingConstructor` deployment failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] constructor failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// A script whose `setup` reverts must fail after deployment, with the
/// execution trace dumped to the traces directory.
#[test]
fn exec_fails_when_setup_reverts() {
    let err = command(REVERTING_SETUP)
        .run()
        .expect_err("exec must fail when setup reverts");
    assert_eq!(
        err.to_string(),
        "script contract `ScriptWithRevertingSetup` setup failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] setup failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// A script whose `exec` reverts must fail with the execution trace dumped
/// to the traces directory.
#[test]
fn exec_fails_when_exec_reverts() {
    let err = command(REVERTING_EXEC)
        .run()
        .expect_err("exec must fail when exec reverts");
    assert_eq!(
        err.to_string(),
        "script contract `ScriptWithRevertingExec` exec failed"
    );

    assert!(
        traces_contain(".ripfuzz/traces", "[revert] exec failed"),
        "a dumped trace must contain the revert reason"
    );
}

/// An explicit contract name must select that contract instead of the
/// default name derived from the file stem.
#[test]
fn exec_uses_explicit_contract_name() {
    command(CUSTOM_NAME)
        .run()
        .expect("exec should run the explicitly named contract");
}

/// A script path that exists but names a missing contract must fail with
/// the available contracts listed.
#[test]
fn exec_fails_when_contract_name_is_wrong() {
    let err = command(WRONG_NAME)
        .run()
        .expect_err("exec must fail for a wrong contract name");
    assert_eq!(
        err.to_string(),
        "contract `DoesNotExist` not found in `fixtures/executor/script-deployment/ScriptCustomName.sol`, available contracts: CustomTarget, ScriptCustomName"
    );
}

/// A script path that does not exist must fail before compilation.
#[test]
fn exec_fails_when_script_file_is_missing() {
    let err = command(MISSING_FILE)
        .run()
        .expect_err("exec must fail for a missing file");
    assert_eq!(
        err.to_string(),
        "script file `fixtures/executor/script-deployment/ScriptMissing.sol` not found"
    );
}

/// Whether any dumped trace file under the directory contains the needle.
///
/// Tests run in parallel and share the traces directory, so the most recent
/// trace file may belong to another test.
fn traces_contain(dir: &str, needle: &str) -> bool {
    fs::read_dir(dir)
        .expect("traces dir must exist")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .any(|entry| {
            fs::read_to_string(entry.path())
                .map(|trace| trace.contains(needle))
                .unwrap_or(false)
        })
}
