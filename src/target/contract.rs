//! Target contract definition and validation.

use alloy_json_abi::{Function, StateMutability};
use anyhow::{Result, bail, ensure};
use revm::primitives::Bytes;

use crate::foundry::{BuildArtifact, BuildArtifactId};

/// A validated target contract ready for fuzzing.
///
/// Created from a Foundry [`BuildArtifact`] by validating that it represents a
/// concrete contract and extracting the functions the fuzzer will exercise.
#[derive(Debug, Clone)]
pub struct Contract {
    /// The unique build artifact identifier.
    pub artifact_id: BuildArtifactId,
    /// Functions the fuzzer will call to mutate state.
    pub target_functions: Vec<Function>,
    /// Invariant functions checked after every call sequence.
    pub invariant_functions: Vec<Function>,
    /// Optional setup function called once after deployment.
    pub setup_function: Option<Function>,
    /// Initcode used to deploy the contract.
    pub initcode: Bytes,
}

impl TryFrom<BuildArtifact> for Contract {
    type Error = anyhow::Error;

    fn try_from(artifact: BuildArtifact) -> Result<Self> {
        let contract = match artifact {
            BuildArtifact::Contract(c) => c,
            _ => bail!("target artifact must be a concrete contract"),
        };

        let artifact_id = contract.id;
        let initcode: Bytes = contract.bytecode.object.parse().unwrap_or_default();

        let all_functions: Vec<Function> = contract.abi.functions().cloned().collect();

        let mut target_functions = Vec::new();
        let mut invariant_functions = Vec::new();
        let mut setup_function = None;

        for func in all_functions {
            if func.name == "setUp" {
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

#[cfg(test)]
mod tests {
    use super::*;

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
