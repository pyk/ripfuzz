//! Dependency fetcher - downloads and extracts dependency archives.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use multihash::Multihash;
use sha2::{Digest, Sha256};
use tar::Archive;
use tracing::info;

/// SHA2-256 multihash code from the multihash table.
const SHA2_256_CODE: u64 = 0x12;

/// Reject downloads larger than 256 MiB.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// A downloaded dependency archive with its multihash digest.
///
/// The hash is the sha2-256 digest wrapped in the multihash envelope and
/// hex encoded with a `0x` prefix (e.g. `0x1220abc...`), matching the
/// `build.zig.zon` package hash format.
#[derive(Clone, Debug)]
pub struct Download {
    bytes: Vec<u8>,
    hash: String,
}

impl Download {
    /// Multihash of the archive contents.
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

/// Fetches a dependency archive from a tarball URL.
///
/// Archives are installed under `{root}/.ripfuzz/dependencies/{name}`.
#[derive(Clone, Debug)]
pub struct Fetcher {
    name: String,
    url: String,
    root: PathBuf,
}

impl Fetcher {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            root: PathBuf::from("."),
        }
    }

    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = root.as_ref().to_path_buf();
        self
    }

    /// Directory the dependency is installed into.
    pub fn dir(&self) -> PathBuf {
        self.root
            .join(".ripfuzz")
            .join("dependencies")
            .join(&self.name)
    }

    /// Downloads the archive from the configured URL.
    pub fn download(&self) -> Result<Download> {
        // 1. Download the archive, rejecting responses above the size limit.
        info!("downloading dependency {} from {}", self.name, self.url);
        let mut response = ureq::get(&self.url)
            .call()
            .with_context(|| format!("failed to download dependency `{}`", self.name))?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_ARCHIVE_BYTES)
            .read_to_vec()
            .with_context(|| format!("failed to read archive for dependency `{}`", self.name))?;

        // 2. Hash the raw archive bytes as a sha2-256 multihash.
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let multihash = Multihash::<64>::wrap(SHA2_256_CODE, &digest)
            .context("failed to wrap archive digest in a multihash")?;
        let hash = format!("0x{}", hex::encode(multihash.to_bytes()));

        Ok(Download { bytes, hash })
    }

    /// Extracts a downloaded archive into `{root}/.ripfuzz/dependencies/{name}`.
    ///
    /// Archives that pack a single root directory (GitHub tarballs) are
    /// stripped, so `{name}/src/std.sol` resolves directly against the
    /// extracted tree. An existing installation is replaced.
    pub fn install(&self, download: &Download) -> Result<()> {
        // 1. Reject names that could escape the dependencies directory.
        self.ensure_valid_name()?;

        // 2. Extract the archive into a staging directory so a failed or
        //    rejected download never clobbers an existing installation.
        let dir = self.dir();
        let dependencies_dir = dir
            .parent()
            .context("dependency directory has no parent")?
            .to_path_buf();
        let staging = dependencies_dir.join(format!(".{}-staging", self.name));
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .with_context(|| format!("failed to remove {}", staging.display()))?;
        }
        fs::create_dir_all(&staging)
            .with_context(|| format!("failed to create {}", staging.display()))?;
        extract_archive(&download.bytes, &staging)?;

        // 3. Collapse a single root directory (GitHub archives).
        let source = collapse_root(&staging)?;

        // 4. Replace any existing installation.
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to remove {}", dir.display()))?;
        }

        // 5. Move the staged tree into place and drop the staging leftovers.
        fs::rename(&source, &dir)
            .with_context(|| format!("failed to install into {}", dir.display()))?;
        if staging.exists()
            && let Err(err) = fs::remove_dir_all(&staging)
        {
            info!("failed to clean up {}: {err}", staging.display());
        }

        info!("dependency {} installed to {}", self.name, dir.display());
        Ok(())
    }

    fn ensure_valid_name(&self) -> Result<()> {
        let valid = !self.name.is_empty()
            && self.name != "."
            && self.name != ".."
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        ensure!(
            valid,
            "invalid dependency name `{}`: use letters, digits, `-`, or `_`",
            self.name
        );
        Ok(())
    }
}

/// Extracts a tar.gz archive into `dest`.
///
/// Regular files and directories are extracted, symlinks and other entry
/// types are skipped, and paths that could escape `dest` are rejected.
fn extract_archive(bytes: &[u8], dest: impl AsRef<Path>) -> Result<()> {
    let dest = dest.as_ref();
    let gz = GzDecoder::new(bytes);
    let mut archive = Archive::new(gz);
    for entry in archive
        .entries()
        .context("failed to read archive entries")?
    {
        let mut entry = entry.context("failed to read archive entry")?;

        // 1. Read the entry path and reject paths that could escape the
        //    destination. The tar crate validates paths only when creating
        //    archives, not when reading them.
        let path = entry
            .path()
            .context("failed to read archive entry path")?
            .to_path_buf();
        ensure_safe_path(&path)?;

        // 2. Extract directories and regular files, skipping symlinks and
        //    other entry types.
        let target = dest.join(&path);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
        } else if entry.header().entry_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            entry
                .unpack(&target)
                .with_context(|| format!("failed to extract {}", target.display()))?;
        }
    }
    Ok(())
}

/// Rejects absolute paths and parent-directory components, which would let a
/// crafted archive write outside the extraction directory.
fn ensure_safe_path(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    ensure!(
        !path.is_absolute(),
        "archive contains absolute path `{}`",
        path.display()
    );
    ensure!(
        !path
            .components()
            .any(|component| component == Component::ParentDir),
        "archive contains unsafe path `{}`",
        path.display()
    );
    Ok(())
}

/// Returns the extracted tree when the archive packs a single root directory,
/// or the staging directory itself for flat archives.
fn collapse_root(staging: &Path) -> Result<PathBuf> {
    let mut entries =
        fs::read_dir(staging).with_context(|| format!("failed to read {}", staging.display()))?;
    let first = entries
        .next()
        .transpose()
        .context("failed to read archive contents")?;
    if let Some(first) = first
        && entries.next().is_none()
        && first.path().is_dir()
    {
        return Ok(first.path());
    }
    Ok(staging.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::GzEncoder};

    use super::*;

    /// Builds a tar.gz archive in memory from `(path, content)` entries.
    ///
    /// Entry paths are written into the raw header name field, so the helper
    /// can produce archives the `tar` crate would refuse to create with
    /// `append_data` (traversal paths), which the extraction path must
    /// reject.
    fn tarball(entries: &[(&str, &str)]) -> Vec<u8> {
        // 1. Build the plain tar bytes, appending each entry under a safe
        //    placeholder name.
        let mut builder = tar::Builder::new(Vec::new());
        for (idx, (_, content)) in entries.iter().enumerate() {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("entry-{idx}"), content.as_bytes())
                .unwrap();
        }
        let mut bytes = builder.into_inner().unwrap();

        // 2. Walk the entry blocks and patch each header name field with the
        //    real entry path, then refresh the checksum, which covers the
        //    name bytes.
        let mut offset = 0;
        for (path, content) in entries {
            let header = &mut bytes[offset..offset + 512];
            header[..100].fill(0);
            header[..path.len()].copy_from_slice(path.as_bytes());
            header[148..156].copy_from_slice(b"        ");
            let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
            header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
            offset += 512 + content.len().div_ceil(512) * 512;
        }

        // 3. Gzip the patched archive.
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn install_strips_single_root_directory() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("ripfuzz", "unused").with_root(dir.path());
        let download = Download {
            bytes: tarball(&[
                ("ripfuzz-std-main/src/std.sol", "// std\n"),
                ("ripfuzz-std-main/src/Harness.sol", "// harness\n"),
            ]),
            hash: "0x1220".to_owned(),
        };

        fetcher.install(&download).unwrap();

        assert!(fetcher.dir().join("src/std.sol").is_file());
        assert!(fetcher.dir().join("src/Harness.sol").is_file());
    }

    #[test]
    fn install_keeps_flat_archives() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("lib", "unused").with_root(dir.path());
        let download = Download {
            bytes: tarball(&[("src/std.sol", "// std\n"), ("README.md", "# readme\n")]),
            hash: "0x1220".to_owned(),
        };

        fetcher.install(&download).unwrap();

        assert!(fetcher.dir().join("src/std.sol").is_file());
        assert!(fetcher.dir().join("README.md").is_file());
    }

    #[test]
    fn install_replaces_existing_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("ripfuzz", "unused").with_root(dir.path());
        let first = Download {
            bytes: tarball(&[("ripfuzz-std-main/src/old.sol", "// old\n")]),
            hash: "0x1220".to_owned(),
        };
        let second = Download {
            bytes: tarball(&[("ripfuzz-std-main/src/new.sol", "// new\n")]),
            hash: "0x1220".to_owned(),
        };

        fetcher.install(&first).unwrap();
        fetcher.install(&second).unwrap();

        assert!(fetcher.dir().join("src/new.sol").is_file());
        assert!(!fetcher.dir().join("src/old.sol").exists());
    }

    #[test]
    fn install_rejects_unsafe_names() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("../escape", "unused").with_root(dir.path());
        let download = Download {
            bytes: tarball(&[("src/std.sol", "// std\n")]),
            hash: "0x1220".to_owned(),
        };

        let err = fetcher.install(&download).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid dependency name `../escape`: use letters, digits, `-`, or `_`"
        );
    }

    #[test]
    fn install_rejects_parent_directory_entries() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("lib", "unused").with_root(dir.path());
        let download = Download {
            bytes: tarball(&[("../escape.sol", "// escape\n")]),
            hash: "0x1220".to_owned(),
        };

        let err = fetcher.install(&download).unwrap_err();

        assert_eq!(
            err.to_string(),
            "archive contains unsafe path `../escape.sol`"
        );
        assert!(!dir.path().join("escape.sol").exists());
    }

    #[test]
    fn install_rejects_absolute_entries() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new("lib", "unused").with_root(dir.path());
        let download = Download {
            bytes: tarball(&[("/tmp/escape.sol", "// escape\n")]),
            hash: "0x1220".to_owned(),
        };

        let err = fetcher.install(&download).unwrap_err();

        assert_eq!(
            err.to_string(),
            "archive contains absolute path `/tmp/escape.sol`"
        );
        assert!(!dir.path().join("tmp").join("escape.sol").exists());
    }
}
