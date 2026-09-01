//! Configuration for `ripfuzz`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use solc::EvmVersion;

fn default_out() -> PathBuf {
    PathBuf::from(".ripfuzz/solc")
}

fn default_evm_version() -> EvmVersion {
    EvmVersion::Prague
}

fn default_optimizer_runs() -> usize {
    200
}

/// Configuration loaded from `ripfuzz.toml`.
///
/// ```toml
/// [solc]
/// version = "0.8.36"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Solc compiler configuration.
    pub solc: SolcConfig,

    /// Root used to resolve the config path. Not part of the TOML file.
    #[serde(skip)]
    root: PathBuf,
}

/// Solc section of `ripfuzz.toml`.
///
/// ```toml
/// [solc]
/// version = "0.8.36"
/// out = ".ripfuzz/solc"
/// evm_version = "cancun"
/// optimizer = true
/// optimizer_runs = 200
/// via_ir = true
/// remappings = [
///     "@openzeppelin/=lib/openzeppelin-contracts/",
///     "@uniswap/=node_modules/@uniswap/",
/// ]
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolcConfig {
    /// Solc version to use for compilation. Ripfuzz does not detect the
    /// version automatically.
    pub version: String,

    /// Output directory for solc compilation artifacts. Defaults to
    /// `.ripfuzz/solc` relative to the project root.
    #[serde(default = "default_out")]
    pub out: PathBuf,

    /// Target EVM version for compilation. This affects which opcodes are
    /// available. Defaults to `prague`.
    #[serde(default = "default_evm_version")]
    pub evm_version: EvmVersion,

    /// Enable the optimizer. Defaults to `false`.
    #[serde(default)]
    pub optimizer: bool,

    /// Number of optimizer runs. Defaults to `200`.
    #[serde(default = "default_optimizer_runs")]
    pub optimizer_runs: usize,

    /// Enable the IR-based compilation pipeline for better optimization.
    /// Defaults to `false`.
    #[serde(default)]
    pub via_ir: bool,

    /// Map import paths to actual file locations. Takes precedence over
    /// remappings with the same prefix in `{root}/remappings.txt`.
    #[serde(default)]
    pub remappings: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            solc: SolcConfig::new(),
            root: PathBuf::from("."),
        }
    }

    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = root.as_ref().to_path_buf();
        self
    }

    pub fn load(&self, path: impl AsRef<Path>) -> Result<Self> {
        let path = self.join(path.as_ref());
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config `{}`", path.display()))?;
        let mut config = Self::parse(&content)?;
        config.root = self.root.clone();
        Ok(config)
    }

    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse config")
    }

    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

impl SolcConfig {
    pub fn new() -> Self {
        Self {
            version: String::new(),
            out: default_out(),
            evm_version: default_evm_version(),
            optimizer: false,
            optimizer_runs: default_optimizer_runs(),
            via_ir: false,
            remappings: Vec::new(),
        }
    }
}

impl Default for SolcConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn load_resolves_relative_path_against_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("ripfuzz.toml"),
            "[solc]\nversion = \"0.8.36\"\n",
        )
        .unwrap();

        let config = Config::new()
            .with_root(dir.path())
            .load("ripfuzz.toml")
            .unwrap();

        assert_eq!(config.solc.version, "0.8.36");
        assert_eq!(config.solc.out, PathBuf::from(".ripfuzz/solc"));
    }

    #[test]
    fn load_missing_file_fails_with_resolved_path() {
        let dir = tempfile::tempdir().unwrap();

        let err = Config::new()
            .with_root(dir.path())
            .load("missing.toml")
            .unwrap_err();

        let expected = dir.path().join("missing.toml");
        assert_eq!(
            err.to_string(),
            format!("failed to read config `{}`", expected.display())
        );
    }

    #[test]
    fn parse_uses_documented_defaults() {
        let config = Config::parse("[solc]\nversion = \"0.8.36\"\n").unwrap();

        assert_eq!(
            config,
            Config {
                solc: SolcConfig {
                    version: "0.8.36".to_owned(),
                    out: PathBuf::from(".ripfuzz/solc"),
                    evm_version: EvmVersion::Prague,
                    optimizer: false,
                    optimizer_runs: 200,
                    via_ir: false,
                    remappings: Vec::new(),
                },
                root: PathBuf::from(""),
            }
        );
    }

    #[test]
    fn parse_full_config() {
        let config = Config::parse(
            r#"
[solc]
version = "0.8.36"
out = ".ripfuzz/solc"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
via_ir = true
remappings = [
    "@openzeppelin/=lib/openzeppelin-contracts/",
    "@uniswap/=node_modules/@uniswap/",
]
"#,
        )
        .unwrap();

        assert_eq!(config.solc.version, "0.8.36");
        assert_eq!(config.solc.out, PathBuf::from(".ripfuzz/solc"));
        assert_eq!(config.solc.evm_version, EvmVersion::Cancun);
        assert!(config.solc.optimizer);
        assert_eq!(config.solc.optimizer_runs, 200);
        assert!(config.solc.via_ir);
        assert_eq!(
            config.solc.remappings,
            vec![
                "@openzeppelin/=lib/openzeppelin-contracts/".to_owned(),
                "@uniswap/=node_modules/@uniswap/".to_owned(),
            ]
        );
    }

    #[test]
    fn parse_rejects_legacy_flat_solc_field() {
        let err = Config::parse("solc = \"0.8.36\"\n").unwrap_err();

        assert_eq!(err.to_string(), "failed to parse config");
        assert_eq!(
            err.root_cause().to_string(),
            "TOML parse error at line 1, column 8\n  |\n1 | solc = \"0.8.36\"\n  |        ^^^^^^^^\ninvalid type: string \"0.8.36\", expected struct SolcConfig\n"
        );
    }

    #[test]
    fn parse_requires_solc_section() {
        let err = Config::parse("").unwrap_err();

        assert_eq!(err.to_string(), "failed to parse config");
        assert_eq!(
            err.root_cause().to_string(),
            "TOML parse error at line 1, column 1\n  |\n1 | \n  | ^\nmissing field `solc`\n"
        );
    }

    #[test]
    fn parse_requires_solc_version() {
        let err = Config::parse("[solc]\n").unwrap_err();

        assert_eq!(err.to_string(), "failed to parse config");
        assert_eq!(
            err.root_cause().to_string(),
            "TOML parse error at line 1, column 1\n  |\n1 | [solc]\n  | ^^^^^^\nmissing field `version`\n"
        );
    }

    #[test]
    fn parse_rejects_unknown_solc_fields() {
        let err = Config::parse("[solc]\nversion = \"0.8.36\"\nfoo = 1\n").unwrap_err();

        assert_eq!(err.to_string(), "failed to parse config");
        assert_eq!(
            err.root_cause().to_string(),
            "TOML parse error at line 3, column 1\n  |\n3 | foo = 1\n  | ^^^\nunknown field `foo`, expected one of `version`, `out`, `evm_version`, `optimizer`, `optimizer_runs`, `via_ir`, `remappings`\n"
        );
    }
}
