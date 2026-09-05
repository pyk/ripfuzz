//! Unique identifier for one command run.
//!
//! [`RunId`] is minted once per command invocation and shared across every
//! per-run artifact (logs, traces, statistics, coverage) so files from the
//! same campaign share their filename stem.
//!
//! ```rust
//! use ripfuzz::cli::RunId;
//!
//! let run = RunId::new();
//! println!("run: {run}");
//! ```

use std::fmt;

/// Unique identifier for one command run, `{unix-timestamp}-{id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunId(String);

impl RunId {
    /// Mint a new run id from the current timestamp and a random id.
    pub fn new() -> Self {
        let timestamp = jiff::Timestamp::now().as_second();
        let uuid: String = uuid::Uuid::new_v4().into();
        let id = uuid.split('-').next().unwrap_or_default();
        Self(format!("{timestamp}-{id}"))
    }

    /// The filename stem shared by every artifact of this run.
    pub fn stem(&self) -> &str {
        &self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_generates_unique_stems() {
        assert_ne!(RunId::new().stem(), RunId::new().stem());
    }

    #[test]
    fn stem_has_timestamp_and_id_parts() {
        let stem = RunId::new().stem().to_owned();
        let (timestamp, id) = stem.split_once('-').unwrap();

        assert!(timestamp.parse::<i64>().is_ok());
        assert_eq!(id.len(), 8);
    }

    #[test]
    fn display_matches_stem() {
        let run = RunId::new();

        assert_eq!(run.to_string(), run.stem());
    }
}
