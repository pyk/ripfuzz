//! Solc installer - downloads and verifies static binaries.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::info;

/// Ensures a `solc` binary for a given version is installed.
#[derive(Clone, Debug)]
pub struct SolcInstaller {
    version: String,
}

impl SolcInstaller {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
        }
    }

    pub fn binary_path(&self) -> PathBuf {
        PathBuf::from(".ripfuzz/bin").join(format!("solc-{}", self.version))
    }

    pub fn ensure_installed(&self) -> Result<()> {
        let binary_path = self.binary_path();

        // 1. Reuse the binary when it is already installed.
        if binary_path.is_file() {
            info!("using existing solc {}", self.version);
            return Ok(());
        }

        // 2. Detect the platform and fetch the release list.
        let platform = detect_platform()?;
        let list_url = format!("https://binaries.soliditylang.org/{platform}/list.json");

        info!("downloading solc {} list for {platform}", self.version);

        let mut list_resp = ureq::get(&list_url)
            .call()
            .with_context(|| format!("failed to fetch solc list for {platform}"))?;
        let list_text = list_resp
            .body_mut()
            .read_to_string()
            .context("failed to read solc list")?;

        let list: SolcList =
            serde_json::from_str(&list_text).context("failed to parse solc list")?;

        // 3. Find the release build for the requested version.
        let build = list
            .builds
            .iter()
            .find(|b| b.version == self.version)
            .with_context(|| format!("solc version {} not found for {platform}", self.version))?;

        // 4. Download the binary.
        let bin_url = format!(
            "https://binaries.soliditylang.org/{platform}/{}",
            build.path.display()
        );

        info!("downloading solc {} from {bin_url}", self.version);

        let mut bin_resp = ureq::get(&bin_url)
            .call()
            .with_context(|| format!("failed to download solc {}", self.version))?;

        let bytes = bin_resp
            .body_mut()
            .with_config()
            .limit(50 * 1024 * 1024)
            .read_to_vec()
            .context("failed to read solc binary")?;

        // 5. Verify the sha256 checksum.
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = hasher.finalize();
        let hash_hex = format!("0x{}", hex::encode(hash));
        let expected = build.sha256.to_lowercase();
        let got = hash_hex.to_lowercase();
        ensure!(
            got == expected,
            "sha256 mismatch for solc {}: expected {expected}, got {got}",
            self.version
        );

        // 6. Save the binary and make it executable.
        if let Some(parent) = binary_path.parent() {
            fs::create_dir_all(parent).context("failed to create bin dir")?;
        }

        fs::write(&binary_path, &bytes)
            .with_context(|| format!("failed to write {}", binary_path.display()))?;

        #[cfg(unix)]
        {
            let mut perm = fs::metadata(&binary_path)?.permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&binary_path, perm)?;
        }

        info!("saved solc {} to {}", self.version, binary_path.display());

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct SolcList {
    builds: Vec<SolcBuild>,
}

#[derive(Debug, Deserialize)]
struct SolcBuild {
    path: PathBuf,
    version: String,
    sha256: String,
}

fn detect_platform() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let platform = match (os, arch) {
        ("linux", "x86_64") => "linux-amd64",
        ("linux", "x86") => "linux-amd64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-arm64",
        ("macos", "x86_64") => "macosx-amd64",
        ("macos", "aarch64") => "macosx-amd64",
        _ => bail!("unsupported platform {os}/{arch} for solc download"),
    };
    Ok(platform.to_owned())
}
