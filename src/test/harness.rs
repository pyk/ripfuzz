//! Validated harness for the `test` command.
//!
//! [`TestHarness`] turns the raw [`SolcOutput`](ripfuzz::compilers::solc::SolcOutput) into
//! a structured harness by extracting the target contract and checking the
//! test harness rules against its ABI, resolving the functions used by later
//! steps such as deployment and fuzzing.
//!
//! Rules:
//!
//! - MUST be a deployable contract
//! - MUST have a constructor without arguments and not `payable`
//! - MAY have a `setup` function without arguments and not `payable`
//! - MAY have a `summary` function without arguments and not `payable`
//! - MAY have `invariant_*` functions without arguments and not `payable`
//!
//! ```rust
//! use ripfuzz::test::TestHarness;
//!
//! // let solc_output = solc.compile()?;
//! // let test_harness = TestHarness::try_from(&solc_output)?;
//! // chain.deploy(&test_harness)?;
//! ```

use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Context, Result, ensure};

use crate::compilers::solc::SolcOutput;
use crate::evm::DeployInput;
use crate::harness::HarnessId;

/// A compiled harness validated and structured for the `test` command.
#[derive(Debug, Clone)]
pub struct TestHarness {
    id: HarnessId,
    abi: JsonAbi,
    initcode: String,
    setup: Option<Function>,
    summary: Option<Function>,
    invariants: Vec<Function>,
    handlers: Vec<Function>,
}

impl TryFrom<&SolcOutput> for TestHarness {
    type Error = anyhow::Error;

    fn try_from(solc_output: &SolcOutput) -> Result<Self> {
        // 1. Extract the target contract from the compilation output.
        let id = &solc_output.id;
        let contracts = solc_output
            .output
            .contracts
            .get(&id.path)
            .with_context(|| {
                format!(
                    "harness source `{}` not found in compilation output",
                    id.path.display()
                )
            })?;
        let contract = contracts.get(&id.name).with_context(|| {
            let mut names: Vec<String> = contracts.keys().map(|name| name.to_owned()).collect();
            names.sort();
            format!(
                "contract `{}` not found in `{}`, available contracts: {}",
                id.name,
                id.path.display(),
                names.join(", ")
            )
        })?;

        // 2. Decode the ABI JSON into the alloy JSON ABI representation.
        let abi = contract
            .abi
            .as_ref()
            .context("harness ABI missing from compilation output")?;
        let abi = serde_json::to_value(&abi.items).context("failed to serialize harness ABI")?;
        let abi: JsonAbi = serde_json::from_value(abi).context("failed to decode harness ABI")?;

        // 3. Extract the initcode and reject contracts without bytecode.
        let initcode = contract
            .evm
            .as_ref()
            .and_then(|evm| evm.bytecode.as_ref())
            .and_then(|bytecode| bytecode.object.as_ref())
            .context("harness initcode missing from compilation output")?;
        ensure!(
            !initcode.is_empty(),
            "harness contract `{}` has empty initcode",
            id
        );

        // 4. Reject a constructor with arguments or payable.
        if let Some(constructor) = &abi.constructor {
            ensure!(
                constructor.inputs.is_empty(),
                "test harness `{}` constructor must not take arguments",
                id
            );
            ensure!(
                constructor.state_mutability != StateMutability::Payable,
                "test harness `{}` constructor must not be `payable`",
                id
            );
        }

        // 5. Capture the optional `setup` and `summary` functions and check
        //    their arguments and mutability.
        let setup = capture_optional(id, &abi, "setup")?;
        let summary = capture_optional(id, &abi, "summary")?;

        // 6. Capture the `invariant_*` functions, sorted by name for stable
        //    reporting, and check their arguments and mutability.
        let mut invariants: Vec<Function> = abi
            .functions()
            .filter(|function| function.name.starts_with("invariant_"))
            .cloned()
            .collect();
        invariants.sort_by(|a, b| a.name.cmp(&b.name));
        for function in &invariants {
            ensure!(
                function.inputs.is_empty(),
                "test harness `{}` function `{}` must not take arguments",
                id,
                function.name
            );
            ensure!(
                function.state_mutability != StateMutability::Payable,
                "test harness `{}` function `{}` must not be `payable`",
                id,
                function.name
            );
        }

        // 7. Capture the fuzzable handler functions, excluding `setup`,
        //    `summary`, and `invariant_*`.
        let handlers = abi
            .functions()
            .filter(|function| {
                function.name != "setup"
                    && function.name != "summary"
                    && !function.name.starts_with("invariant_")
            })
            .cloned()
            .collect();

        Ok(Self {
            id: id.clone(),
            abi,
            initcode: initcode.clone(),
            setup,
            summary,
            invariants,
            handlers,
        })
    }
}

/// Resolve an optional no-argument, non-payable harness function by name.
fn capture_optional(id: &HarnessId, abi: &JsonAbi, name: &str) -> Result<Option<Function>> {
    let Some(function) = abi.function(name).and_then(|functions| functions.first()) else {
        return Ok(None);
    };
    ensure!(
        function.inputs.is_empty(),
        "test harness `{}` function `{}` must not take arguments",
        id,
        name
    );
    ensure!(
        function.state_mutability != StateMutability::Payable,
        "test harness `{}` function `{}` must not be `payable`",
        id,
        name
    );
    Ok(Some(function.clone()))
}

impl TestHarness {
    /// The harness identifier.
    pub fn id(&self) -> &HarnessId {
        &self.id
    }

    /// The harness ABI.
    pub fn abi(&self) -> &JsonAbi {
        &self.abi
    }

    /// The optional `setup` function run before fuzzing.
    pub fn setup(&self) -> Option<&Function> {
        self.setup.as_ref()
    }

    /// The optional `summary` function run after fuzzing.
    pub fn summary(&self) -> Option<&Function> {
        self.summary.as_ref()
    }

    /// The `invariant_*` functions checked after each handler call.
    pub fn invariants(&self) -> &[Function] {
        &self.invariants
    }

    /// The fuzzable handler functions, excluding `setup`, `summary`, and
    /// `invariant_*`.
    pub fn handlers(&self) -> &[Function] {
        &self.handlers
    }
}

impl From<&TestHarness> for DeployInput {
    fn from(test_harness: &TestHarness) -> Self {
        DeployInput::new(&test_harness.initcode)
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::JsonAbi;
    use serde_json::json;

    use super::*;

    const ID: &str = "src/Harness.sol:Harness";

    fn solc_output(contracts: serde_json::Value) -> SolcOutput {
        SolcOutput {
            id: HarnessId::try_from(ID).unwrap(),
            output: serde_json::from_value(json!({"contracts": contracts})).unwrap(),
        }
    }

    fn solc_output_with(functions: &[&str]) -> SolcOutput {
        let abi = JsonAbi::parse(functions.iter().copied()).unwrap();
        solc_output(json!({
            "src/Harness.sol": {
                "Harness": {
                    "abi": serde_json::to_value(&abi).unwrap(),
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }))
    }

    #[test]
    fn accepts_valid_harness() {
        let test_harness =
            TestHarness::try_from(&solc_output_with(&["function deposit(uint256)"])).unwrap();

        assert_eq!(test_harness.id().name, "Harness");
        assert_eq!(test_harness.setup(), None);
        assert_eq!(test_harness.summary(), None);
        assert!(test_harness.invariants().is_empty());
        assert_eq!(test_harness.handlers().len(), 1);
    }

    #[test]
    fn captures_setup_summary_and_invariants() {
        let test_harness = TestHarness::try_from(&solc_output_with(&[
            "function setup()",
            "function deposit(uint256)",
            "function summary() view",
            "function invariant_total() view",
            "function invariant_zero() view",
        ]))
        .unwrap();

        assert_eq!(test_harness.setup().map(|f| f.name.as_str()), Some("setup"));
        assert_eq!(
            test_harness.summary().map(|f| f.name.as_str()),
            Some("summary")
        );
        let invariants: Vec<&str> = test_harness
            .invariants()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(invariants, vec!["invariant_total", "invariant_zero"]);
        let handlers: Vec<&str> = test_harness
            .handlers()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(handlers, vec!["deposit"]);
    }

    #[test]
    fn constructor_with_args_fails() {
        let solc_output = solc_output(json!({
            "src/Harness.sol": {
                "Harness": {
                    "abi": [{"type": "constructor", "inputs": [{"name": "x", "type": "uint256"}], "stateMutability": "nonpayable"}],
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` constructor must not take arguments"
        );
    }

    #[test]
    fn payable_constructor_fails() {
        let solc_output = solc_output(json!({
            "src/Harness.sol": {
                "Harness": {
                    "abi": [{"type": "constructor", "inputs": [], "stateMutability": "payable"}],
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` constructor must not be `payable`"
        );
    }

    #[test]
    fn setup_with_args_fails() {
        let solc_output = solc_output_with(&["function setup(uint256)"]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `setup` must not take arguments"
        );
    }

    #[test]
    fn payable_setup_fails() {
        let solc_output = solc_output_with(&["function setup() payable"]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `setup` must not be `payable`"
        );
    }

    #[test]
    fn summary_with_args_fails() {
        let solc_output = solc_output_with(&["function summary(uint256)"]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `summary` must not take arguments"
        );
    }

    #[test]
    fn payable_summary_fails() {
        let solc_output = solc_output_with(&["function summary() payable"]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `summary` must not be `payable`"
        );
    }

    #[test]
    fn invariant_with_args_fails() {
        let solc_output = solc_output_with(&[
            "function invariant_total(uint256) view",
            "function deposit()",
        ]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `invariant_total` must not take arguments"
        );
    }

    #[test]
    fn payable_invariant_fails() {
        let solc_output =
            solc_output_with(&["function invariant_total() payable", "function deposit()"]);

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "test harness `src/Harness.sol:Harness` function `invariant_total` must not be `payable`"
        );
    }

    #[test]
    fn missing_source_fails() {
        let solc_output = solc_output(json!({}));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness source `src/Harness.sol` not found in compilation output"
        );
    }

    #[test]
    fn missing_contract_lists_alternatives() {
        let solc_output = solc_output(json!({
            "src/Harness.sol": {"Alpha": {}, "Beta": {}}
        }));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "contract `Harness` not found in `src/Harness.sol`, available contracts: Alpha, Beta"
        );
    }

    #[test]
    fn missing_abi_fails() {
        let solc_output = solc_output(json!({
            "src/Harness.sol": {"Harness": {}}
        }));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness ABI missing from compilation output"
        );
    }

    #[test]
    fn empty_initcode_fails() {
        let solc_output = solc_output(json!({
            "src/Harness.sol": {
                "Harness": {
                    "abi": [],
                    "evm": {"bytecode": {"object": ""}}
                }
            }
        }));

        let err = TestHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness contract `src/Harness.sol:Harness` has empty initcode"
        );
    }

    #[test]
    fn deploy_input_uses_initcode() {
        let solc_output = solc_output_with(&["function deposit(uint256)"]);
        let test_harness = TestHarness::try_from(&solc_output).unwrap();

        let deploy_input: DeployInput = (&test_harness).into();
        assert_eq!(deploy_input.initcode, "0x6080");
    }
}
