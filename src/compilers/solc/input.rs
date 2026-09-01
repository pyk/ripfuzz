//! Standard JSON input building for solc compilation.
//!
//! Assembles the resolved sources and remappings into the solc standard JSON
//! input with ripfuzz's output selection.

use std::collections::HashMap;
use std::path::PathBuf;

use solc::{EvmVersion, Optimizer, OutputSelector, StandardJSONInput};

/// Builds the solc standard JSON input for a compilation.
#[derive(Clone, Debug, Default)]
pub struct StandardJSONInputBuilder {
    sources: HashMap<PathBuf, String>,
    remappings: Vec<String>,
    evm_version: Option<EvmVersion>,
    optimizer: Option<(bool, usize)>,
    via_ir: Option<bool>,
}

impl StandardJSONInputBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_sources(mut self, sources: HashMap<PathBuf, String>) -> Self {
        self.sources = sources;
        self
    }

    pub fn with_remappings(mut self, remappings: Vec<String>) -> Self {
        self.remappings = remappings;
        self
    }

    /// Sets the target EVM version for code generation.
    pub fn with_evm_version(mut self, evm_version: EvmVersion) -> Self {
        self.evm_version = Some(evm_version);
        self
    }

    /// Enables the optimizer and sets the number of runs.
    pub fn with_optimizer(mut self, enabled: bool, runs: usize) -> Self {
        self.optimizer = Some((enabled, runs));
        self
    }

    /// Enables the IR-based compilation pipeline.
    pub fn with_via_ir(mut self, via_ir: bool) -> Self {
        self.via_ir = Some(via_ir);
        self
    }

    /// Assembles the standard JSON input.
    ///
    /// Remappings are only set on the compiler settings when present, so a
    /// project without remappings gets a settings object without them. The
    /// optimizer, EVM version, and via-IR settings are only set when
    /// configured, so solc's own defaults stay in effect otherwise.
    pub fn build(self) -> StandardJSONInput {
        let mut input = StandardJSONInput::new();
        for (path, content) in self.sources {
            input = input.add_source(path, content);
        }
        if !self.remappings.is_empty() {
            input.settings.remappings = Some(self.remappings);
        }
        if let Some((enabled, runs)) = self.optimizer {
            input.settings.optimizer = Some(Optimizer {
                enabled: Some(enabled),
                runs: Some(runs),
                details: None,
            });
        }
        if let Some(evm_version) = self.evm_version {
            input.settings.evm_version = Some(evm_version);
        }
        if let Some(via_ir) = self.via_ir {
            input.settings.via_ir = Some(via_ir);
        }
        input.output_selection(
            vec![
                OutputSelector::Abi,
                OutputSelector::Metadata,
                OutputSelector::StorageLayout,
                OutputSelector::EvmBytecodeObject,
                OutputSelector::EvmBytecodeSourceMap,
                OutputSelector::EvmBytecodeLinkReferences,
                OutputSelector::EvmDeployedBytecodeObject,
                OutputSelector::EvmDeployedBytecodeSourceMap,
                OutputSelector::EvmDeployedBytecodeLinkReferences,
                OutputSelector::EvmDeployedBytecodeImmutableReferences,
                OutputSelector::EvmMethodIdentifiers,
            ],
            vec![OutputSelector::Ast],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sets_remappings_only_when_present() {
        let mut sources = HashMap::new();
        sources.insert(
            PathBuf::from("Harness.sol"),
            "contract Harness {}".to_owned(),
        );

        let input = StandardJSONInputBuilder::new()
            .with_sources(sources.clone())
            .with_remappings(vec!["ripfuzz/=lib/ripfuzz/src/".to_owned()])
            .build();
        assert_eq!(
            input.settings.remappings,
            Some(vec!["ripfuzz/=lib/ripfuzz/src/".to_owned()])
        );
        assert_eq!(input.sources.len(), 1);

        let input = StandardJSONInputBuilder::new()
            .with_sources(sources)
            .build();
        assert_eq!(input.settings.remappings, None);
    }

    #[test]
    fn build_sets_compiler_settings_only_when_configured() {
        let input = StandardJSONInputBuilder::new().build();
        assert_eq!(input.settings.optimizer, None);
        assert_eq!(input.settings.evm_version, None);
        assert_eq!(input.settings.via_ir, None);

        let input = StandardJSONInputBuilder::new()
            .with_optimizer(true, 200)
            .with_evm_version(EvmVersion::Cancun)
            .with_via_ir(true)
            .build();
        assert_eq!(
            input.settings.optimizer,
            Some(Optimizer {
                enabled: Some(true),
                runs: Some(200),
                details: None,
            })
        );
        assert_eq!(input.settings.evm_version, Some(EvmVersion::Cancun));
        assert_eq!(input.settings.via_ir, Some(true));
    }
}
