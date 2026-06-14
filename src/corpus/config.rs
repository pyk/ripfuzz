//! Builder configuration for [`SharedCorpus`](super::SharedCorpus).

use std::path::{Path, PathBuf};

use crate::corpus::extractor::ExtractedLiterals;

/// Builder configuration for [`SharedCorpus`](super::SharedCorpus).
///
/// # Example
///
/// ```no_run
/// use std::path::PathBuf;
/// use raptor::CorpusConfig;
/// use raptor::SharedCorpus;
/// use raptor::ExtractedLiterals;
///
/// let corpus_dir = PathBuf::from("/tmp/corpus");
/// let functions = vec![];
/// let literals = ExtractedLiterals::default();
/// let config = CorpusConfig::new(corpus_dir)
///     .handler_functions(functions)
///     .max_calls(32)
///     .literals(literals);
/// let corpus = SharedCorpus::new(config);
/// ```
#[derive(Debug, Clone)]
pub struct CorpusConfig {
    pub corpus_dir: PathBuf,
    pub handler_functions: Vec<alloy_json_abi::Function>,
    pub max_calls_length: usize,
    pub literals: ExtractedLiterals,
}

impl CorpusConfig {
    /// Start building a [`CorpusConfig`] with the required corpus directory.
    pub fn new(corpus_dir: impl AsRef<Path>) -> Self {
        Self {
            corpus_dir: corpus_dir.as_ref().to_path_buf(),
            handler_functions: Vec::new(),
            max_calls_length: 100,
            literals: ExtractedLiterals::default(),
        }
    }

    /// Set the handler functions used for corpus generation and mutation.
    pub fn handler_functions(mut self, functions: Vec<alloy_json_abi::Function>) -> Self {
        self.handler_functions = functions;
        self
    }

    /// Set the maximum number of calls per generated sequence.
    pub fn max_calls(mut self, n: usize) -> Self {
        self.max_calls_length = n;
        self
    }

    /// Set the extracted literals used for random value generation.
    pub fn literals(mut self, literals: ExtractedLiterals) -> Self {
        self.literals = literals;
        self
    }
}
