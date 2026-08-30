//! Standard JSON input building for solc compilation.
//!
//! Assembles the resolved sources and remappings into the solc standard JSON
//! input with ripfuzz's output selection.

use std::collections::HashMap;
use std::path::PathBuf;

use solc::{OutputSelector, StandardJSONInput};

/// Builds the solc standard JSON input for a compilation.
#[derive(Clone, Debug, Default)]
pub struct StandardJSONInputBuilder {
    sources: HashMap<PathBuf, String>,
    remappings: Vec<String>,
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

    /// Assembles the standard JSON input.
    ///
    /// Remappings are only set on the compiler settings when present, so a
    /// project without remappings gets a settings object without them.
    pub fn build(self) -> StandardJSONInput {
        let mut input = StandardJSONInput::new();
        for (path, content) in self.sources {
            input = input.add_source(path, content);
        }
        if !self.remappings.is_empty() {
            input.settings.remappings = Some(self.remappings);
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
}
