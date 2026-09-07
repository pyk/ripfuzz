//! `compile` CLI command implementation.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing::{info, warn};

use crate::cli::HarnessId;
use crate::compilers::solc::Solc;
use crate::config::Config;
use crate::logger::Logger;

/// Compile a contract and print warnings.
#[derive(Debug, Parser)]
pub struct Command {
    /// Contract file to compile, e.g. `src/Counter.sol` or `src/Counter.sol:Counter`.
    #[arg(value_name = "CONTRACT")]
    pub contract: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub root: PathBuf,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Log verbosity level.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: tracing::Level,
}

impl Command {
    /// Run the `compile` command.
    pub fn run(&self) -> Result<()> {
        // 1. Initialize logging.
        Logger::new()
            .with_root(&self.root)
            .with_quiet(self.quiet)
            .with_level(self.log_level)
            .init()?;

        // 2. Load configuration relative to the project root.
        let root = self.root.clone();
        let config = Config::new().with_root(&root).load(&self.config)?;

        // 3. Compile the contract via Solc relative to the project root.
        let remappings = config.compile_remappings();
        let solc_output = Solc::new()
            .with_version(&config.solc.version)
            .with_root(&root)
            .with_target(&self.contract.path)
            .with_name(&self.contract.name)
            .with_out(&config.solc.out)
            .with_evm_version(config.solc.evm_version)
            .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs)
            .with_via_ir(config.solc.via_ir)
            .with_remappings(remappings)
            .compile()?;

        // 4. Validate the target contract exists in the compilation output.
        solc_output.contract()?;

        // 5. Log solc warnings when enabled, so diagnostics that do not
        //    fail the build stay visible only when the user opts in.
        if config.solc.show_warning {
            let warnings = solc_output.warnings();
            for warning in &warnings {
                warn!("{warning}");
            }
            if !warnings.is_empty() {
                info!(
                    "compilation successful with {} warning{}",
                    warnings.len(),
                    if warnings.len() == 1 { "" } else { "s" }
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use solc::{ErrorComponent, ErrorType, Severity};

    use super::*;
    use crate::compilers::solc::SolcOutput;

    fn diagnostic(severity: Severity, message: &str, formatted: Option<&str>) -> solc::Error {
        solc::Error {
            source_location: None,
            secondary_source_locations: None,
            error_code: None,
            r#type: ErrorType::Warning,
            component: ErrorComponent::General,
            severity,
            message: message.to_owned(),
            formatted_message: formatted.map(ToOwned::to_owned),
        }
    }

    fn output_with(errors: Vec<solc::Error>) -> SolcOutput {
        SolcOutput {
            id: HarnessId::try_from("src/Counter.sol:Counter").unwrap(),
            output: solc::StandardJSONOutput {
                errors: Some(errors),
                ..Default::default()
            },
        }
    }

    #[test]
    fn warnings_returns_only_warnings_with_formatted_messages() {
        let output = output_with(vec![
            diagnostic(
                Severity::Warning,
                "unused parameter",
                Some("Warning: Unused function parameter."),
            ),
            diagnostic(
                Severity::Error,
                "expected `;`",
                Some("Error: Expected `;`."),
            ),
            diagnostic(Severity::Info, "note", Some("Info: Just a note.")),
        ]);

        assert_eq!(
            output.warnings(),
            vec!["Warning: Unused function parameter."]
        );
    }

    #[test]
    fn warnings_falls_back_to_the_message_without_formatting() {
        let output = output_with(vec![diagnostic(
            Severity::Warning,
            "unused parameter",
            None,
        )]);

        assert_eq!(output.warnings(), vec!["unused parameter"]);
    }

    #[test]
    fn warnings_is_empty_without_diagnostics() {
        let output = SolcOutput {
            id: HarnessId::try_from("src/Counter.sol:Counter").unwrap(),
            output: solc::StandardJSONOutput {
                errors: None,
                ..Default::default()
            },
        };

        assert_eq!(output.warnings(), Vec::<&str>::new());
    }
}
