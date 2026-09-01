//! Harness contract definition and validation.

use std::collections::HashMap;
use std::path::PathBuf;

use alloy_dyn_abi::{DynSolType, Specifier};
use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Context, Result, bail, ensure};

use crate::evm::DeployLibraryInput;
use crate::foundry::{Artifact, ArtifactId, ContractArtifact};

/// A validated harness contract ready for fuzzing.
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
    pub handler_functions: Vec<Function>,
    /// Invariant functions checked after every call sequence.
    pub invariant_functions: Vec<Function>,
    /// Max functions whose return values are maximized in max mode.
    pub max_functions: Vec<Function>,
    /// Optional setup function called once after deployment.
    pub setup_function: Option<Function>,
    /// Hex-encoded initcode used to deploy the contract.
    pub initcode: String,
    /// Linked libraries that must be deployed before the harness contract.
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
                "harness contract must not have duplicate function names: `{}`",
                name
            );
        }

        let all_functions: Vec<Function> = contract.abi.functions().cloned().collect();

        let mut handler_functions = Vec::new();
        let mut invariant_functions = Vec::new();
        let mut max_functions = Vec::new();
        let mut setup_function = None;

        for func in all_functions {
            if func.name == "setup" {
                setup_function = Some(func);
                continue;
            }

            if func.name == "summary" {
                ensure!(
                    func.inputs.is_empty(),
                    "summary function must have no arguments, but has {}",
                    func.inputs.len()
                );
                ensure!(
                    !matches!(
                        func.state_mutability,
                        StateMutability::Pure | StateMutability::View
                    ),
                    "summary function must not be view or pure"
                );
                continue;
            }

            if func.name.starts_with("invariant_") {
                ensure!(
                    func.inputs.is_empty(),
                    "invariant function `{}` must have no arguments, but has {}",
                    func.name,
                    func.inputs.len()
                );
                invariant_functions.push(func);
                continue;
            }

            if func.name.starts_with("max_") {
                ensure!(
                    func.inputs.is_empty(),
                    "max function `{}` must have no arguments, but has {}",
                    func.name,
                    func.inputs.len()
                );
                ensure!(
                    func.outputs.len() == 1
                        && func.outputs[0].resolve().ok() == Some(DynSolType::Uint(256)),
                    "max function `{}` must return a single uint256 value",
                    func.name
                );
                ensure!(
                    matches!(
                        func.state_mutability,
                        StateMutability::View | StateMutability::Pure
                    ),
                    "max function `{}` must be view or pure",
                    func.name
                );
                max_functions.push(func);
                continue;
            }

            handler_functions.push(func);
        }

        if let Some(constructor) = &contract.abi.constructor {
            ensure!(
                constructor.inputs.is_empty(),
                "harness contract constructor must not have arguments"
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
            !handler_functions.is_empty(),
            "harness contract must have at least one handler function"
        );

        Ok(Self {
            artifact_id,
            abi: contract.abi.clone(),
            handler_functions,
            invariant_functions,
            max_functions,
            setup_function,
            initcode,
            libraries,
        })
    }

    /// The optional `summary()` function for logging a final summary in the
    /// traced re-run, when the harness declares one.
    pub fn summary_function(&self) -> Option<&Function> {
        self.abi.functions.get("summary").and_then(|f| f.first())
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

    /// Load a harness contract from the build artifacts and prepare its library
    /// dependencies.
    ///
    /// `artifact_id` must be a concrete contract (not an interface, library, or
    /// abstract contract).
    pub fn try_get(
        build_artifacts: &HashMap<ArtifactId, Artifact>,
        artifact_id: &ArtifactId,
    ) -> Result<Self> {
        let artifact = build_artifacts.get(artifact_id).with_context(|| {
            format!(
                "target artifact `{}` not found in build artifacts",
                artifact_id
            )
        })?;
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
}
