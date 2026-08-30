//! Validated harness for the `max` command.
//!
//! [`MaxHarness`] turns a generic [`Harness`] into a structured harness by
//! checking the max harness rules against its ABI and resolving the functions
//! used by later steps such as deployment and maximization.
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
//! // let max_harness = MaxHarness::try_from(harness)?;
//! // chain.deploy(&max_harness)?;
//! ```

use alloy_json_abi::{Function, StateMutability};
use anyhow::{Context, Result, ensure};

use crate::evm::DeployInput;
use crate::harness::{Harness, HarnessId};

/// A compiled harness validated and structured for the `max` command.
#[derive(Debug, Clone)]
pub struct MaxHarness {
    harness: Harness,
    value: Function,
    setup: Option<Function>,
    summary: Option<Function>,
}

impl TryFrom<Harness> for MaxHarness {
    type Error = anyhow::Error;

    fn try_from(harness: Harness) -> Result<Self> {
        // 1. Reject `invariant_*` functions.
        let invariants: Vec<&str> = harness
            .abi
            .functions()
            .filter(|function| function.name.starts_with("invariant_"))
            .map(|function| function.name.as_str())
            .collect();
        ensure!(
            invariants.is_empty(),
            "max harness `{}` must not define `invariant_*` functions, found: {}",
            harness.id,
            invariants.join(", ")
        );

        // 2. Resolve the `value` function and reject overloads.
        let candidates = harness.abi.function("value").with_context(|| {
            format!(
                "max harness `{}` must define a `value` function",
                harness.id
            )
        })?;
        ensure!(
            candidates.len() == 1,
            "max harness `{}` must define exactly one `value` function, found {}",
            harness.id,
            candidates.len()
        );
        let value = candidates[0].clone();

        // 3. Check the `value` mutability and return type.
        ensure!(
            matches!(
                value.state_mutability,
                StateMutability::View | StateMutability::Pure
            ),
            "max harness `{}` function `value` must be `view` or `pure`, found `{}`",
            harness.id,
            value.state_mutability.as_json_str()
        );
        ensure!(
            value.outputs.len() == 1,
            "max harness `{}` function `value` must return exactly one value",
            harness.id
        );
        ensure!(
            value.outputs[0].ty == "uint256",
            "max harness `{}` function `value` must return `uint256`, found `{}`",
            harness.id,
            value.outputs[0].ty
        );

        // 4. Capture the optional `setup` and `summary` functions.
        let setup = harness
            .abi
            .function("setup")
            .and_then(|functions| functions.first())
            .cloned();
        let summary = harness
            .abi
            .function("summary")
            .and_then(|functions| functions.first())
            .cloned();

        Ok(Self {
            harness,
            value,
            setup,
            summary,
        })
    }
}

impl MaxHarness {
    /// The harness identifier.
    pub fn id(&self) -> &HarnessId {
        &self.harness.id
    }

    /// The harness ABI.
    pub fn abi(&self) -> &alloy_json_abi::JsonAbi {
        &self.harness.abi
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
}

impl From<&MaxHarness> for DeployInput {
    fn from(max_harness: &MaxHarness) -> Self {
        DeployInput::new(&max_harness.harness.initcode)
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::JsonAbi;

    use super::*;

    const ID: &str = "src/Harness.sol:Harness";

    fn harness(functions: &[&str]) -> Harness {
        let abi = JsonAbi::parse(functions.iter().copied()).unwrap();
        Harness {
            id: HarnessId::try_from(ID).unwrap(),
            abi,
            initcode: "0x6080".to_owned(),
        }
    }

    #[test]
    fn accepts_valid_harness() {
        let max_harness =
            MaxHarness::try_from(harness(&["function value() view returns (uint256)"])).unwrap();

        assert_eq!(max_harness.id().name, "Harness");
        assert_eq!(max_harness.value().name, "value");
        assert_eq!(max_harness.setup(), None);
        assert_eq!(max_harness.summary(), None);
    }

    #[test]
    fn accepts_pure_value() {
        MaxHarness::try_from(harness(&["function value() pure returns (uint256)"])).unwrap();
    }

    #[test]
    fn captures_setup_and_summary() {
        let max_harness = MaxHarness::try_from(harness(&[
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
    fn missing_value_fails() {
        let err = MaxHarness::try_from(harness(&["function set()"])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must define a `value` function"
        );
    }

    #[test]
    fn overloaded_value_fails() {
        let err = MaxHarness::try_from(harness(&[
            "function value() view returns (uint256)",
            "function value(uint256) view returns (uint256)",
        ]))
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must define exactly one `value` function, found 2"
        );
    }

    #[test]
    fn nonpayable_value_fails() {
        let err =
            MaxHarness::try_from(harness(&["function value() returns (uint256)"])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must be `view` or `pure`, found `nonpayable`"
        );
    }

    #[test]
    fn payable_value_fails() {
        let err = MaxHarness::try_from(harness(&["function value() payable returns (uint256)"]))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must be `view` or `pure`, found `payable`"
        );
    }

    #[test]
    fn multiple_value_outputs_fail() {
        let err = MaxHarness::try_from(harness(&[
            "function value() view returns (uint256, uint256)",
        ]))
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must return exactly one value"
        );
    }

    #[test]
    fn wrong_value_output_type_fails() {
        let err = MaxHarness::try_from(harness(&["function value() view returns (uint128)"]))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` function `value` must return `uint256`, found `uint128`"
        );
    }

    #[test]
    fn invariant_function_fails() {
        let err = MaxHarness::try_from(harness(&[
            "function value() view returns (uint256)",
            "function invariant_neverFails() view",
        ]))
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "max harness `src/Harness.sol:Harness` must not define `invariant_*` functions, found: invariant_neverFails"
        );
    }

    #[test]
    fn deploy_input_uses_initcode() {
        let max_harness =
            MaxHarness::try_from(harness(&["function value() view returns (uint256)"])).unwrap();

        let deploy_input: DeployInput = (&max_harness).into();
        assert_eq!(deploy_input.initcode, "0x6080");
    }
}
