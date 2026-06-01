//! Target contract definition and validation.

use std::collections::HashMap;
use std::path::PathBuf;

use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Context, Result, bail, ensure};

use crate::evm::DeployLibraryInput;
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
    /// Full contract ABI (includes all functions).
    pub abi: JsonAbi,
    /// Functions the fuzzer will call to mutate state.
    pub target_functions: Vec<Function>,
    /// Invariant functions checked after every call sequence.
    pub invariant_functions: Vec<Function>,
    /// Optional setup function called once after deployment.
    pub setup_function: Option<Function>,
    /// Hex-encoded initcode used to deploy the contract.
    pub initcode: String,
    /// Linked libraries that must be deployed before the target contract.
    pub libraries: Vec<DeployLibraryInput>,
}

impl Contract {
    fn from_contract_artifact(
        contract: &ContractArtifact,
        libraries: Vec<DeployLibraryInput>,
    ) -> Result<Self> {
        let artifact_id = contract.id.clone();
        let initcode = contract.bytecode.object.clone();

        for (name, funcs) in &contract.abi.functions {
            ensure!(
                funcs.len() <= 1,
                "target contract must not have duplicate function names: `{}`",
                name
            );
        }

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
            abi: contract.abi.clone(),
            target_functions,
            invariant_functions,
            setup_function,
            initcode,
            libraries,
        })
    }

    /// Build a tree of [`DeployLibraryInput`] from an artifact's library dependencies.
    fn build_libraries(
        artifact: &Artifact,
        build_artifacts: &HashMap<ArtifactId, Artifact>,
    ) -> Result<Vec<DeployLibraryInput>> {
        let deps = match artifact {
            Artifact::Contract(c) => c.bytecode.library_dependencies(),
            Artifact::Library(c) => c.bytecode.library_dependencies(),
            _ => return Ok(Vec::new()),
        };

        let mut libraries = Vec::new();
        for (file, names) in deps {
            for name in names {
                let identifier = format!("{}:{}", file, name);

                let temp_id = ArtifactId {
                    path: PathBuf::from(&file),
                    name,
                };
                let lib_artifact = build_artifacts
                    .get(&temp_id)
                    .with_context(|| format!("library artifact missing: {}", identifier))?;

                let initcode = match lib_artifact {
                    Artifact::Library(c) => c.bytecode.object.as_str(),
                    _ => bail!("artifact {} is not a library", identifier),
                };

                let nested = Self::build_libraries(lib_artifact, build_artifacts)?;
                let mut lib_input = DeployLibraryInput::new(identifier, initcode);
                for nested_lib in nested {
                    lib_input = lib_input.add_library(nested_lib);
                }
                libraries.push(lib_input);
            }
        }
        Ok(libraries)
    }

    /// Load a target contract from the build artifacts and prepare its library
    /// dependencies.
    ///
    /// `artifact_id` must be a concrete contract (not an interface, library, or
    /// abstract contract).
    pub fn try_get(
        build_artifacts: &HashMap<ArtifactId, Artifact>,
        artifact_id: &ArtifactId,
    ) -> Result<Self> {
        let artifact = build_artifacts
            .get(artifact_id)
            .with_context(|| format!("target artifact `{}` not found", artifact_id))?;
        let contract = match artifact {
            Artifact::Contract(c) => c,
            _ => bail!("target artifact must be a concrete contract"),
        };
        let libraries = Self::build_libraries(artifact, build_artifacts)?;
        Self::from_contract_artifact(contract, libraries)
    }
}

#[cfg(test)]
mod tests {
    use revm::primitives::Bytes;

    use super::*;
    use crate::foundry::Project;

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
        let project = Project::new("fixtures/target-contract-validation");
        let artifacts = project.load_artifacts()?;
        let id = ArtifactId::try_from(contract_id)?;
        Contract::try_get(&artifacts, &id)
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

    // -----------------------------------------------------------------------
    // 11. Duplicate function names are not allowed
    // -----------------------------------------------------------------------

    #[test]
    fn duplicate_function_name_fails() {
        let err = load_fixture("src/InvalidDuplicateFunctionName.sol:InvalidDuplicateFunctionName")
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("target contract must not have duplicate function names")
        );
    }
}
