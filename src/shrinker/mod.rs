//! Per-thread shrinker that minimizes a failing corpus item.

pub use shrinker::{Config, Shrinker, ShrinkerOutput};

#[allow(clippy::module_inception)]
mod shrinker;
