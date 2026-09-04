//! `fetch` CLI command implementation.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use crate::config::{Config, Dependency};
use crate::dependencies::Fetcher;
use crate::logger::Logger;

const CONFIG_FILE: &str = "ripfuzz.toml";

/// Fetch and install a dependency from a tarball URL.
#[derive(Debug, Parser)]
pub struct Command {
    /// Name to register the dependency under.
    #[arg(value_name = "NAME")]
    pub name: String,

    /// URL of the tar.gz archive to fetch.
    #[arg(value_name = "URL")]
    pub url: String,
}

impl Command {
    pub fn run(&self) -> Result<()> {
        // 1. Initialize stderr logging without a log file so command errors
        //    reach the console without creating `.ripfuzz` state.
        Logger::new().with_root(".").disable_log_file().init()?;

        // 2. Load the config so a recorded hash can be verified and the
        //    fetched dependency recorded afterwards.
        let root = PathBuf::from(".");
        let config = Config::new().with_root(&root).load(CONFIG_FILE)?;

        // 3. Download the dependency archive and hash it.
        let fetcher = Fetcher::new(&self.name, &self.url).with_root(&root);
        let download = fetcher.download()?;
        info!("dependency {} hash {}", self.name, download.hash());

        // 4. Report a changed hash instead of erroring, so re-running `fetch`
        //    against a moving URL updates the pin. The rewritten config in
        //    step 6 records the downloaded hash.
        if let Some(existing) = config.dependencies.get(&self.name)
            && existing.hash != download.hash()
        {
            info!(
                "dependency {} hash changed from {} to {}, updating {}",
                self.name,
                existing.hash,
                download.hash(),
                CONFIG_FILE
            );
        }

        // 5. Extract the archive into `.ripfuzz/dependencies/<name>`.
        fetcher.install(&download)?;

        // 6. Record the dependency in `ripfuzz.toml`.
        let dependency = Dependency {
            url: self.url.clone(),
            hash: download.hash().to_owned(),
        };
        Config::record_dependency(CONFIG_FILE, &self.name, dependency)?;
        info!("dependency {} recorded in {}", self.name, CONFIG_FILE);

        Ok(())
    }
}
