//! Validated script for the `exec` command.
//!
//! [`Script`] turns the raw [`SolcOutput`](ripfuzz::compilers::solc::SolcOutput) into
//! a structured script by extracting the target contract and checking the exec
//! script rules against its ABI, resolving the functions used by later steps
//! such as deployment and execution.
//!
//! Rules:
//!
//! - MUST have an `exec` function
//! - MAY have a `setup` function
//! - MAY have a constructor
//!
//! ```rust
//! use ripfuzz::executor::Script;
//!
//! // let solc_output = solc.compile()?;
//! // let script = Script::try_from(&solc_output)?;
//! // chain.deploy(&script)?;
//! ```

use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Context, Result, ensure};

use crate::compilers::solc::SolcOutput;
use crate::evm::DeployInput;
use crate::harness::HarnessId;

/// A compiled script validated and structured for the `exec` command.
#[derive(Debug, Clone)]
pub struct Script {
    id: HarnessId,
    initcode: String,
    exec: Function,
    setup: Option<Function>,
}

impl TryFrom<&SolcOutput> for Script {
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
                    "script source `{}` not found in compilation output",
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
            .context("script ABI missing from compilation output")?;
        let abi = serde_json::to_value(&abi.items).context("failed to serialize script ABI")?;
        let abi: JsonAbi = serde_json::from_value(abi).context("failed to decode script ABI")?;

        // 3. Extract the initcode and reject contracts without bytecode.
        let initcode = contract
            .evm
            .as_ref()
            .and_then(|evm| evm.bytecode.as_ref())
            .and_then(|bytecode| bytecode.object.as_ref())
            .context("script initcode missing from compilation output")?;
        ensure!(
            !initcode.is_empty(),
            "script contract `{}` has empty initcode",
            id
        );

        // 4. Check the constructor. An implicit constructor is absent from the
        //    ABI, so only explicit definitions are validated here.
        if let Some(constructor) = &abi.constructor {
            ensure!(
                constructor.inputs.is_empty(),
                "script contract `{}` constructor must take no arguments",
                id
            );
            ensure!(
                !matches!(constructor.state_mutability, StateMutability::Payable),
                "script contract `{}` constructor must not be `payable`",
                id
            );
        }

        // 5. Capture the optional `setup` function and reject invalid
        //    definitions, including overloads.
        if let Some(setups) = abi.function("setup") {
            for setup in setups {
                ensure!(
                    setup.inputs.is_empty(),
                    "script contract `{}` function `setup` must take no arguments",
                    id
                );
                ensure!(
                    matches!(setup.state_mutability, StateMutability::NonPayable),
                    "script contract `{}` function `setup` must be `external` or `public`, found `{}`",
                    id,
                    setup.state_mutability.as_json_str()
                );
            }
        }
        let setup = abi
            .function("setup")
            .and_then(|functions| functions.first())
            .cloned();

        // 6. Resolve the `exec` function and reject overloads.
        let candidates = abi
            .function("exec")
            .with_context(|| format!("script contract `{}` must define an `exec` function", id))?;
        ensure!(
            candidates.len() == 1,
            "script contract `{}` must define exactly one `exec` function, found {}",
            id,
            candidates.len()
        );
        let exec = candidates[0].clone();

        // 7. Check the `exec` arguments and mutability. Internal and private
        //    functions are absent from the ABI, so the remaining invalid
        //    mutabilities are `payable`, `view`, and `pure`.
        ensure!(
            exec.inputs.is_empty(),
            "script contract `{}` function `exec` must take no arguments",
            id
        );
        ensure!(
            matches!(exec.state_mutability, StateMutability::NonPayable),
            "script contract `{}` function `exec` must be `external` or `public`, found `{}`",
            id,
            exec.state_mutability.as_json_str()
        );

        Ok(Self {
            id: id.clone(),
            initcode: initcode.clone(),
            exec,
            setup,
        })
    }
}

impl Script {
    /// The script identifier.
    pub fn id(&self) -> &HarnessId {
        &self.id
    }

    /// The `exec` function run once after deployment and setup.
    pub fn exec(&self) -> &Function {
        &self.exec
    }

    /// The optional `setup` function run after deployment.
    pub fn setup(&self) -> Option<&Function> {
        self.setup.as_ref()
    }
}

impl From<&Script> for DeployInput {
    fn from(script: &Script) -> Self {
        DeployInput::new(&script.initcode)
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::JsonAbi;
    use serde_json::json;

    use super::*;

    const ID: &str = "src/Script.sol:Script";

    fn solc_output(contracts: serde_json::Value) -> SolcOutput {
        SolcOutput {
            id: HarnessId::try_from(ID).unwrap(),
            output: serde_json::from_value(json!({"contracts": contracts})).unwrap(),
        }
    }

    fn solc_output_with(functions: &[&str]) -> SolcOutput {
        let abi = JsonAbi::parse(functions.iter().copied()).unwrap();
        solc_output(json!({
            "src/Script.sol": {
                "Script": {
                    "abi": serde_json::to_value(&abi).unwrap(),
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }))
    }

    #[test]
    fn accepts_valid_script() {
        let script = Script::try_from(&solc_output_with(&["function exec()"])).unwrap();

        assert_eq!(script.id().name, "Script");
        assert_eq!(script.exec().name, "exec");
        assert_eq!(script.setup(), None);
    }

    #[test]
    fn captures_setup() {
        let script =
            Script::try_from(&solc_output_with(&["function setup()", "function exec()"])).unwrap();

        assert_eq!(script.setup().map(|f| f.name.as_str()), Some("setup"));
        assert_eq!(script.exec().name, "exec");
    }

    #[test]
    fn missing_exec_fails() {
        let solc_output = solc_output_with(&["function setup()"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` must define an `exec` function"
        );
    }

    #[test]
    fn overloaded_exec_fails() {
        let solc_output = solc_output_with(&["function exec()", "function exec(uint256)"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` must define exactly one `exec` function, found 2"
        );
    }

    #[test]
    fn exec_with_arguments_fails() {
        let solc_output = solc_output_with(&["function exec(uint256)"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` function `exec` must take no arguments"
        );
    }

    #[test]
    fn payable_exec_fails() {
        let solc_output = solc_output_with(&["function exec() payable"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` function `exec` must be `external` or `public`, found `payable`"
        );
    }

    #[test]
    fn setup_with_arguments_fails() {
        let solc_output = solc_output_with(&["function setup(uint256)", "function exec()"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` function `setup` must take no arguments"
        );
    }

    #[test]
    fn payable_setup_fails() {
        let solc_output = solc_output_with(&["function setup() payable", "function exec()"]);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` function `setup` must be `external` or `public`, found `payable`"
        );
    }

    fn solc_output_with_abi(abi: serde_json::Value) -> SolcOutput {
        solc_output(json!({
            "src/Script.sol": {
                "Script": {
                    "abi": abi,
                    "evm": {"bytecode": {"object": "0x6080"}}
                }
            }
        }))
    }

    #[test]
    fn constructor_with_arguments_fails() {
        let abi = JsonAbi::parse(["function exec()"]).unwrap();
        let mut value = serde_json::to_value(&abi).unwrap();
        value
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "constructor", "inputs": [{"name": "", "type": "uint256"}], "stateMutability": "nonpayable"}));
        let solc_output = solc_output_with_abi(value);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` constructor must take no arguments"
        );
    }

    #[test]
    fn payable_constructor_fails() {
        let abi = JsonAbi::parse(["function exec()"]).unwrap();
        let mut value = serde_json::to_value(&abi).unwrap();
        value
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "constructor", "inputs": [], "stateMutability": "payable"}));
        let solc_output = solc_output_with_abi(value);

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` constructor must not be `payable`"
        );
    }

    #[test]
    fn missing_source_fails() {
        let solc_output = solc_output(json!({}));

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script source `src/Script.sol` not found in compilation output"
        );
    }

    #[test]
    fn missing_contract_lists_alternatives() {
        let solc_output = solc_output(json!({
            "src/Script.sol": {"Alpha": {}, "Beta": {}}
        }));

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "contract `Script` not found in `src/Script.sol`, available contracts: Alpha, Beta"
        );
    }

    #[test]
    fn missing_abi_fails() {
        let solc_output = solc_output(json!({
            "src/Script.sol": {"Script": {}}
        }));

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script ABI missing from compilation output"
        );
    }

    #[test]
    fn empty_initcode_fails() {
        let solc_output = solc_output(json!({
            "src/Script.sol": {
                "Script": {
                    "abi": [],
                    "evm": {"bytecode": {"object": ""}}
                }
            }
        }));

        let err = Script::try_from(&solc_output).unwrap_err();
        assert_eq!(
            err.to_string(),
            "script contract `src/Script.sol:Script` has empty initcode"
        );
    }

    #[test]
    fn deploy_input_uses_initcode() {
        let solc_output = solc_output_with(&["function exec()"]);
        let script = Script::try_from(&solc_output).unwrap();

        let deploy_input: DeployInput = (&script).into();
        assert_eq!(deploy_input.initcode, "0x6080");
    }
}
