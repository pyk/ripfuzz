//! Validated harness for the `max` command.
//!
//! [`MaxHarness`] turns the raw [`SolcOutput`](ripfuzz::solc::SolcOutput) into
//! a structured harness by extracting the target contract and checking the max
//! harness rules against its ABI, resolving the functions used by later steps
//! such as deployment and maximization.
//!
//! Rules:
//!
//! - MUST have a `view` or `pure` function `value` that returns `uint256`
//! - MUST NOT define `invariant_*` functions
//! - MAY have a `setup` function
//! - MAY have a `summary` function
//!
//! ```rust
//! use ripfuzz::max::MaxHarness;
//!
//! // let solc_output = solc.compile()?;
//! // let max_harness = MaxHarness::try_from(&solc_output)?;
//! // chain.deploy(&max_harness)?;
//! ```

use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Context, Result, ensure};

use crate::evm::DeployInput;
use crate::harness::HarnessId;
use crate::solc::SolcOutput;

/// A compiled harness validated and structured for the `max` command.
#[derive(Debug, Clone)]
pub struct MaxHarness {
    id: HarnessId,
    abi: JsonAbi,
    initcode: String,
    value: Function,
    setup: Option<Function>,
    summary: Option<Function>,
}

impl TryFrom<&SolcOutput> for MaxHarness {
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

        // 4. Reject `invariant_*` functions.
        let invariants: Vec<&str> = abi
            .functions()
            .filter(|function| function.name.starts_with("invariant_"))
            .map(|function| function.name.as_str())
            .collect();
        ensure!(
            invariants.is_empty(),
            "max harness `{}` must not define `invariant_*` functions, found: {}",
            id,
            invariants.join(", ")
        );

        // 5. Resolve the `value` function and reject overloads.
        let candidates = abi
            .function("value")
            .with_context(|| format!("max harness `{}` must define a `value` function", id))?;
        ensure!(
            candidates.len() == 1,
            "max harness `{}` must define exactly one `value` function, found {}",
            id,
            candidates.len()
        );
        let value = candidates[0].clone();

        // 6. Check the `value` mutability and return type.
        ensure!(
            matches!(
                value.state_mutability,
                StateMutability::View | StateMutability::Pure
            ),
            "max harness `{}` function `value` must be `view` or `pure`, found `{}`",
            id,
            value.state_mutability.as_json_str()
        );
        ensure!(
            value.outputs.len() == 1,
            "max harness `{}` function `value` must return exactly one value",
            id
        );
        ensure!(
            value.outputs[0].ty == "uint256",
            "max harness `{}` function `value` must return `uint256`, found `{}`",
            id,
            value.outputs[0].ty
        );

        // 7. Capture the optional `setup` and `summary` functions.
        let setup = abi
            .function("setup")
            .and_then(|functions| functions.first())
            .cloned();
        let summary = abi
            .function("summary")
            .and_then(|functions| functions.first())
            .cloned();

        Ok(Self {
            id: id.clone(),
            abi,
            initcode: initcode.clone(),
            value,
            setup,
            summary,
        })
    }
}

impl MaxHarness {
    /// The harness identifier.
    pub fn id(&self) -> &HarnessId {
        &self.id
    }

    /// The harness ABI.
    pub fn abi(&self) -> &JsonAbi {
        &self.abi
    }

    /// The `value` function whose return value is maximized.
    pub fn value(&self) -> &Function {
        &self.value
    }

    /// The optional `setup` function run before maximization.
    pub fn setup(&self) -> Option<&Function> {
        self.setup.as_ref()
    }

    /// The optional `summary` function run after maximization.
    pub fn summary(&self) -> Option<&Function> {
        self.summary.as_ref()
    }

    /// The fuzzable handler functions, excluding `value`, `setup`, and
    /// `summary`.
    pub fn handlers(&self) -> Vec<Function> {
        self.abi
            .functions()
            .filter(|function| {
                function.name != "value" && function.name != "setup" && function.name != "summary"
            })
            .cloned()
            .collect()
    }
}

impl From<&MaxHarness> for DeployInput {
    fn from(max_harness: &MaxHarness) -> Self {
        DeployInput::new(&max_harness.initcode)
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
        let max_harness = MaxHarness::try_from(&solc_output_with(&[
            "function value() view returns (uint256)",
        ]))
        .unwrap();

        assert_eq!(max_harness.id().name, "Harness");
        assert_eq!(max_harness.value().name, "value");
        assert_eq!(max_harness.setup(), None);
        assert_eq!(max_harness.summary(), None);
        assert!(max_harness.abi().functions.contains_key("value"));
    }

    #[test]
    fn accepts_pure_value() {
        MaxHarness::try_from(&solc_output_with(&[
            "function value() pure returns (uint256)",
        ]))
        .unwrap();
    }

    #[test]
    fn captures_setup_and_summary() {
        let max_harness = MaxHarness::try_from(&solc_output_with(&[
            "function setup()",
            "function value() view returns (uint256)",
            "function summary() view returns (string memory)",
        ]))
        .unwrap();

        assert_eq!(max_harness.setup().map(|f| f.name.as_str()), Some("setup"));
        assert_eq!(
            max_harness.summary().map(|f| f.name.as_str()),
            Some("summary")
        );
    }

    #[test]
    fn handlers_exclude_value_setup_and_summary() {
        let max_harness = MaxHarness::try_from(&solc_output_with(&[
            "function set(uint256)",
            "function value() view returns (uint256)",
            "function setup()",
            "function summary() view returns (string memory)",
        ]))
        .unwrap();
        let handlers = max_harness.handlers();
        let names: Vec<&str> = handlers
            .iter()
            .map(|function| function.name.as_str())
            .collect();

        assert_eq!(names, vec!["set"]);
    }

    #[test]
    fn missing_value_fails() {
        let solc_output = solc_output_with(&["function set()"]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must define a `value` function"
        );
    }

    #[test]
    fn overloaded_value_fails() {
        let solc_output = solc_output_with(&[
            "function value() view returns (uint256)",
            "function value(uint256) view returns (uint256)",
        ]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must define exactly one `value` function, found 2"
        );
    }

    #[test]
    fn nonpayable_value_fails() {
        let solc_output = solc_output_with(&["function value() returns (uint256)"]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must be `view` or `pure`, found `nonpayable`"
        );
    }

    #[test]
    fn payable_value_fails() {
        let solc_output = solc_output_with(&["function value() payable returns (uint256)"]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must be `view` or `pure`, found `payable`"
        );
    }

    #[test]
    fn multiple_value_outputs_fail() {
        let solc_output = solc_output_with(&["function value() view returns (uint256, uint256)"]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must return exactly one value"
        );
    }

    #[test]
    fn wrong_value_output_type_fails() {
        let solc_output = solc_output_with(&["function value() view returns (uint128)"]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must return `uint256`, found `uint128`"
        );
    }

    #[test]
    fn invariant_function_fails() {
        let solc_output = solc_output_with(&[
            "function value() view returns (uint256)",
            "function invariant_neverFails() view",
        ]);

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must not define `invariant_*` functions, found: invariant_neverFails"
        );
    }

    #[test]
    fn missing_source_fails() {
        let solc_output = solc_output(json!({}));

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
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

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
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

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
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

        let err = MaxHarness::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness contract `src/Harness.sol:Harness` has empty initcode"
        );
    }

    #[test]
    fn deploy_input_uses_initcode() {
        let solc_output = solc_output_with(&["function value() view returns (uint256)"]);
        let max_harness = MaxHarness::try_from(&solc_output).unwrap();

        let deploy_input: DeployInput = (&max_harness).into();
        assert_eq!(deploy_input.initcode, "0x6080");
    }
}
