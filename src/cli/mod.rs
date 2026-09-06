//! CLI command definitions.

use std::path::{Path, absolute};

pub use harness_id::HarnessId;
pub use run_id::RunId;

pub mod compile;
pub mod exec;
pub mod fetch;
pub mod init;
pub mod inspect;
pub mod max;
pub mod test;

mod harness_id;
mod run_id;

/// Default thread count for commands that fuzz across threads.
///
/// Uses the available parallelism of the machine so campaigns scale across
/// all CPU cores by default, falling back to a single thread when the
/// runtime cannot report it.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Render an artifact path relative to the project root for logging.
///
/// Writers return absolute paths, so the root is absolutized before
/// stripping. Relative paths fall back to dot-prefix stripping.
pub fn display_path(root: impl AsRef<Path>, path: impl AsRef<Path>) -> String {
    let root = root.as_ref();
    let path = path.as_ref();
    // 1. Strip the absolutized root from absolute artifact paths, keeping
    //    the absolute path when it escapes the root.
    if path.is_absolute() {
        return absolute(root)
            .ok()
            .and_then(|root| {
                path.strip_prefix(&root)
                    .map(|relative| relative.display().to_string())
                    .ok()
            })
            .unwrap_or_else(|| path.display().to_string());
    }

    // 2. Fall back to dot-prefix stripping for relative paths.
    strip_dot_prefix(path.display().to_string())
}

fn strip_dot_prefix(path: impl AsRef<Path>) -> String {
    let mut display = path.as_ref().display().to_string();
    loop {
        if let Some(stripped) = display.strip_prefix("./") {
            display = stripped.to_owned();
        } else if let Some(stripped) = display.strip_prefix(".\\") {
            display = stripped.to_owned();
        } else {
            break;
        }
    }
    display
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_strips_the_root_from_absolute_paths() {
        let root = Path::new("/proj");
        let path = Path::new("/proj/.ripfuzz/stats/run.json");

        assert_eq!(display_path(root, path), ".ripfuzz/stats/run.json");
    }

    #[test]
    fn display_path_keeps_absolute_paths_outside_the_root() {
        let root = Path::new("/proj");
        let path = Path::new("/tmp/run.json");

        assert_eq!(display_path(root, path), "/tmp/run.json");
    }

    #[test]
    fn display_path_strips_dot_prefix_from_relative_paths() {
        let root = Path::new("/proj");

        assert_eq!(
            display_path(root, Path::new("./.ripfuzz/corpus.json")),
            ".ripfuzz/corpus.json"
        );
        assert_eq!(
            display_path(root, Path::new(".ripfuzz/corpus.json")),
            ".ripfuzz/corpus.json"
        );
    }
}
