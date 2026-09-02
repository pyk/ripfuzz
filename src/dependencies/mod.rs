//! Dependency management for `ripfuzz`.
//!
//! Fetches dependency archives from tarball URLs and installs them under
//! `.ripfuzz/dependencies` so compilation can remap imports into the
//! extracted sources.
//!
//! ```rust
//! use ripfuzz::dependencies::Fetcher;
//!
//! let fetcher = Fetcher::new("ripfuzz", "https://example.com/ripfuzz-std.tar.gz");
//! // let download = fetcher.download()?;
//! // fetcher.install(&download)?;
//! ```

pub use fetch::{Download, Fetcher};

pub mod fetch;
