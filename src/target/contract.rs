//! Target contract definition and validation.

use alloy_json_abi::{Function, StateMutability};
use anyhow::{Result, bail, ensure};
use revm::primitives::Bytes;

use crate::foundry::{Artifact, ArtifactId, ContractArtifact};

/// A validated target contract ready for fuzzing.
///
/// Created from a Foundry [`Artifact`] (or `&Artifact`) by validating that it
/// represents a concrete contract and extracting the functions the fuzzer will
/// exercise.
#[derive(Debug, Clone)]
pub struct Contract {
    /// The unique build artifact identifier.
    pub artifact_id: ArtifactId,
    /// Functions the fuzzer will call to mutate state.
    pub target_functions: Vec<Function>,
    /// Invariant functions checked after every call sequence.
    pub invariant_functions: Vec<Function>,
    /// Optional setup function called once after deployment.
    pub setup_function: Option<Function>,
    /// Initcode used to deploy the contract.
    pub initcode: Bytes,
}

impl Contract {
    fn from_contract_artifact(contract: &ContractArtifact) -> Result<Self> {
        let artifact_id = contract.id.clone();
        let initcode: Bytes = contract.bytecode.object.parse().unwrap_or_default();

        let all_functions: Vec<Function> = contract.abi.functions().cloned().collect();

        let mut target_functions = Vec::new();
        let mut invariant_functions = Vec::new();
        let mut setup_function = None;

        for func in all_functions {
            if func.name == "setup" {
                setup_function = Some(func);
                continue;
            }

            if func.name.starts_with("invariant_")
                && matches!(
                    func.state_mutability,
                    StateMutability::Pure | StateMutability::View
                )
                && func.inputs.is_empty()
            {
                invariant_functions.push(func);
                continue;
            }

            target_functions.push(func);
        }

        if let Some(constructor) = &contract.abi.constructor {
            ensure!(
                constructor.inputs.is_empty(),
                "target contract constructor must not have arguments"
            );
        }

        if let Some(ref func) = setup_function {
            ensure!(
                func.inputs.is_empty(),
                "setup function must not have arguments"
            );
            ensure!(
                !matches!(
                    func.state_mutability,
                    StateMutability::Pure | StateMutability::View
                ),
                "setup function must not be view or pure"
            );
        }

        ensure!(
            !target_functions.is_empty(),
            "target contract must have at least one target function"
        );

        Ok(Self {
            artifact_id,
            target_functions,
            invariant_functions,
            setup_function,
            initcode,
        })
    }
}

impl TryFrom<&Artifact> for Contract {
    type Error = anyhow::Error;

    fn try_from(artifact: &Artifact) -> Result<Self> {
        let contract = match artifact {
            Artifact::Contract(c) => c,
            _ => bail!("target artifact must be a concrete contract"),
        };
        Self::from_contract_artifact(contract)
    }
}

impl TryFrom<Artifact> for Contract {
    type Error = anyhow::Error;

    fn try_from(artifact: Artifact) -> Result<Self> {
        Self::try_from(&artifact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn bytes_from_str_empty() {
        let b: Bytes = "".parse().unwrap_or_default();
        assert!(b.is_empty());
    }

    #[test]
    fn bytes_from_str_with_0x() {
        let b: Bytes = "0x1234".parse().unwrap_or_default();
        assert_eq!(b.len(), 2);
        assert_eq!(b.as_ref(), [0x12, 0x34]);
    }

    #[test]
    fn bytes_from_str_without_0x() {
        let b: Bytes = "1234".parse().unwrap_or_default();
        assert_eq!(b.len(), 2);
        assert_eq!(b.as_ref(), [0x12, 0x34]);
    }

    // -----------------------------------------------------------------------
    // Fixture helpers
    // -----------------------------------------------------------------------

    fn load_fixture(contract_id: &str) -> Result<Contract> {
        let project = crate::foundry::Project::new("fixtures/target-contract-validation");
        let artifacts = project.load_artifacts()?;
        let id = crate::foundry::ArtifactId::try_from(contract_id)?;
        let artifact = artifacts
            .get(&id)
            .context("artifact not found in build artifacts")?;
        Contract::try_from(artifact)
    }

    // -----------------------------------------------------------------------
    // 1. Valid target contract should have >0 target functions
    // -----------------------------------------------------------------------

    #[test]
    fn valid_target_has_target_functions() {
        let contract = load_fixture("src/ValidTarget.sol:ValidTarget").unwrap();
        assert!(!contract.target_functions.is_empty());
        assert!(
            contract
                .target_functions
                .iter()
                .any(|f| f.name == "doSomething")
        );
    }

    // -----------------------------------------------------------------------
    // 2. Valid target contract can have 0 or more invariant functions
    // -----------------------------------------------------------------------

    #[test]
    fn valid_target_can_have_zero_invariants() {
        let contract = load_fixture("src/ValidNoInvariant.sol:ValidNoInvariant").unwrap();
        assert!(contract.invariant_functions.is_empty());
    }

    #[test]
    fn valid_target_can_have_multiple_invariants() {
        let contract =
            load_fixture("src/ValidMultipleInvariants.sol:ValidMultipleInvariants").unwrap();
        assert_eq!(contract.invariant_functions.len(), 2);
        assert!(
            contract
                .invariant_functions
                .iter()
                .any(|f| f.name == "invariant_a")
        );
        assert!(
            contract
                .invariant_functions
                .iter()
                .any(|f| f.name == "invariant_b")
        );
    }

    // -----------------------------------------------------------------------
    // 3. Invariant function must have no arguments
    // -----------------------------------------------------------------------

    #[test]
    fn invariant_with_args_is_not_invariant() {
        let contract =
            load_fixture("src/InvalidInvariantWithArgs.sol:InvalidInvariantWithArgs").unwrap();
        assert!(contract.invariant_functions.is_empty());
        assert!(
            contract
                .target_functions
                .iter()
                .any(|f| f.name == "invariant_check")
        );
    }

    // -----------------------------------------------------------------------
    // 4. Invariant function must be external (implicit: ABI only contains
    //    public/external functions)
    // -----------------------------------------------------------------------

    #[test]
    fn public_invariant_is_classified_as_invariant() {
        let contract = load_fixture("src/ValidTarget.sol:ValidTarget").unwrap();
        assert!(
            contract
                .invariant_functions
                .iter()
                .any(|f| f.name == "invariant_check")
        );
    }

    // -----------------------------------------------------------------------
    // 5. Invariant function must be view or pure
    // -----------------------------------------------------------------------

    #[test]
    fn invariant_non_view_is_not_invariant() {
        let contract =
            load_fixture("src/InvalidInvariantNonView.sol:InvalidInvariantNonView").unwrap();
        assert!(contract.invariant_functions.is_empty());
        assert!(
            contract
                .target_functions
                .iter()
                .any(|f| f.name == "invariant_check")
        );
    }

    // -----------------------------------------------------------------------
    // 6. Constructor must not have arguments
    // -----------------------------------------------------------------------

    #[test]
    fn constructor_with_args_fails() {
        let err = load_fixture("src/InvalidConstructorWithArgs.sol:InvalidConstructorWithArgs")
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("constructor must not have arguments")
        );
    }

    // -----------------------------------------------------------------------
    // 7. setup function is optional
    // -----------------------------------------------------------------------

    #[test]
    fn setup_is_optional() {
        let contract = load_fixture("src/ValidNoInvariant.sol:ValidNoInvariant").unwrap();
        assert!(contract.setup_function.is_none());
    }

    // -----------------------------------------------------------------------
    // 8. setup function must not have arguments
    // -----------------------------------------------------------------------

    #[test]
    fn setup_with_args_fails() {
        let err = load_fixture("src/InvalidSetupWithArgs.sol:InvalidSetupWithArgs").unwrap_err();
        assert!(
            err.to_string()
                .contains("setup function must not have arguments")
        );
    }

    // -----------------------------------------------------------------------
    // 9. setup function must be external (implicit: ABI only contains
    //    public/external functions)
    // -----------------------------------------------------------------------

    #[test]
    fn public_setup_is_accepted() {
        // ValidSetup uses external setup; since ABI omits internal functions,
        // any setup in the ABI is by definition callable externally.
        let contract = load_fixture("src/ValidSetup.sol:ValidSetup").unwrap();
        assert!(contract.setup_function.is_some());
    }

    // -----------------------------------------------------------------------
    // 10. setup function must not be view or pure
    // -----------------------------------------------------------------------

    #[test]
    fn setup_view_fails() {
        let err = load_fixture("src/InvalidSetupView.sol:InvalidSetupView").unwrap_err();
        assert!(
            err.to_string()
                .contains("setup function must not be view or pure")
        );
    }

    #[test]
    fn valid_setup_no_args_succeeds() {
        let contract = load_fixture("src/ValidSetup.sol:ValidSetup").unwrap();
        let setup = contract.setup_function.unwrap();
        assert!(setup.inputs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Edge case: contract with no targets at all fails
    // -----------------------------------------------------------------------

    #[test]
    fn no_targets_fails() {
        let err = load_fixture("src/InvalidNoTargets.sol:InvalidNoTargets").unwrap_err();
        assert!(
            err.to_string()
                .contains("target contract must have at least one target function")
        );
    }
}
