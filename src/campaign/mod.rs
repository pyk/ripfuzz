//! Campaign orchestration: configuration, setup, and result aggregation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use libafl::events::launcher::{ClientDescription, Launcher};
use libafl::events::{EventConfig, SendExiting};
use libafl::monitors::SimpleMonitor;
use libafl_bolts::shmem::{ShMem, ShMemProvider, StdShMemProvider};
use tracing::{debug, error, info, trace, warn};

pub use config::CampaignConfig;

use crate::contract::{ContractArtifact, ContractBuilder};
use crate::worker::{PropertyFailure, Worker, WorkerResult};

pub mod config;
pub mod input;

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub runs: u64,
    pub failures: Vec<PropertyFailure>,
}

/// Builder for constructing a [`Campaign`].
#[derive(Debug)]
pub struct CampaignBuilder {
    contract_path: PathBuf,
    project_path: PathBuf,
    config: CampaignConfig,
}

impl CampaignBuilder {
    /// Override the Foundry project root directory.
    pub fn with_project(mut self, path: impl AsRef<Path>) -> Self {
        self.project_path = path.as_ref().to_path_buf();
        self
    }

    /// Set the fuzzing configuration.
    pub fn with_config(mut self, config: CampaignConfig) -> Self {
        self.config = config;
        self
    }

    /// Build the contract artifact, validate deployment, and generate seeds.
    pub fn build(self) -> Result<Campaign> {
        let artifact = ContractBuilder::build(&self.project_path, &self.contract_path)?;
        info!(contract = %artifact.contract_name, "artifact built");

        // Validate deployment by creating a runner and immediately dropping it.
        let _runner = crate::evm::EvmRunner::from_target(&artifact)?;
        debug!("deployment validated");

        // TODO: review this call squence seed generation
        let seeds = build_seeds(&artifact, self.config.sequence_length);
        let selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .map(|f| f.selector().into())
            .collect();
        debug!(
            seed_count = seeds.len(),
            selector_count = selectors.len(),
            "seeds and selectors generated"
        );

        Ok(Campaign {
            artifact,
            seeds,
            config: self.config,
            selectors,
        })
    }
}

/// A fuzzing campaign that validates a target contract and orchestrates one or more workers.
#[derive(Debug)]
pub struct Campaign {
    artifact: ContractArtifact,
    seeds: Vec<input::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Campaign {
    /// Start building a campaign for the given target contract.
    pub fn for_target(contract: impl AsRef<Path>) -> CampaignBuilder {
        CampaignBuilder {
            contract_path: contract.as_ref().to_path_buf(),
            project_path: PathBuf::new(),
            config: CampaignConfig::default(),
        }
    }

    /// Access the loaded contract artifact.
    pub fn artifact(&self) -> &ContractArtifact {
        &self.artifact
    }

    /// Run the campaign and return an aggregated result.
    pub fn run(&self) -> Result<CampaignResult> {
        let workers = self.config.worker_count();
        let broker_port = self.config.broker_port;

        info!(workers, "starting parallel fuzzing campaign");
        let mut shmem_provider = StdShMemProvider::new()?;
        let mut shmem = shmem_provider.new_shmem(crate::inspector::MAP_SIZE)?;
        shmem.fill(0);
        let map_desc = shmem.description();
        debug!("shared memory allocated for coverage map");

        let artifact = self.artifact.clone();
        let seeds = self.seeds.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();

        let workers_u64 = workers as u64;
        let base_runs = config.max_runs / workers_u64;
        let remainder = (config.max_runs % workers_u64) as usize;
        debug!(base_runs, remainder, "run distribution calculated");

        // Unique identifier so we only collect temp files from this run.
        let campaign_id = format!(
            "{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let campaign_id_for_closure = campaign_id.clone();
        info!(%campaign_id, "campaign id generated");

        let monitor = SimpleMonitor::new(|s: &str| println!("{s}"));

        let run_client = move |state: Option<crate::worker::MyState>,
                               mut mgr: crate::worker::MyMgr,
                               _client: ClientDescription| {
            let client_id = _client.id();
            let core_id = _client.core_id().0;
            let pid = std::process::id();
            let local_max_runs = if core_id < remainder {
                base_runs + 1
            } else {
                base_runs
            };
            info!(client_id, core_id, pid, local_max_runs, "worker started");

            let mut local_provider = StdShMemProvider::new()
                .map_err(|e| libafl::Error::illegal_state(format!("shmem provider failed: {e}")))?;
            let mut local_shmem = local_provider
                .shmem_from_description(map_desc)
                .map_err(|e| libafl::Error::illegal_state(format!("shmem mapping failed: {e}")))?;

            let worker = Worker::new(
                artifact.clone(),
                seeds.clone(),
                config.clone(),
                selectors.clone(),
            );
            let result = worker
                .run(state, &mut mgr, &mut local_shmem, local_max_runs)
                .map_err(|e| libafl::Error::illegal_state(format!("worker run failed: {e}")))?;

            // Persist local results so the campaign can aggregate them.
            let tmp =
                std::env::temp_dir().join(format!("raptor_{campaign_id_for_closure}_{pid}.json"));
            if let Ok(bytes) = serde_json::to_vec(&result) {
                match fs::write(&tmp, &bytes) {
                    Err(e) => warn!(client_id, ?tmp, %e, "failed to write temp file"),
                    Ok(()) => debug!(client_id, ?tmp, "temp file written"),
                }
            }

            // Tell LibAFL that this worker is done so the respawner
            // exits cleanly instead of panicking on a zero exit code.
            debug!(client_id, "calling send_exiting");
            mgr.send_exiting()
                .map_err(|e| libafl::Error::illegal_state(format!("send_exiting failed: {e}")))?;
            info!(client_id, "send_exiting succeeded, worker done");

            // Exit the child process immediately so it does not return
            // through Campaign and print duplicate campaign summaries.
            std::process::exit(0);
        };

        let cores = Self::workers_to_cores(workers)?;

        info!(workers, "spawning parallel fuzzers via Launcher");
        match Launcher::builder()
            .shmem_provider(shmem_provider)
            .monitor(monitor)
            .configuration(EventConfig::from_name("default"))
            .cores(&cores)
            .run_client(run_client)
            .stdout_file(Some("/dev/null"))
            .broker_port(broker_port)
            .build()
            .launch()
        {
            Ok(_) => {
                info!("Launcher exited normally");
            }
            Err(libafl::Error::ShuttingDown) => {
                info!("Launcher returned ShuttingDown (expected after send_exiting)");
            }
            Err(e) => {
                error!(%e, "Launcher failed unexpectedly");
                return Err(e).context("Parallel fuzzing failed");
            }
        }

        // Aggregate worker results from temp files.
        let mut total_runs = 0u64;
        let mut all_failures = Vec::new();
        let tmp_dir = std::env::temp_dir();
        let prefix = format!("raptor_{campaign_id}_");
        info!(%campaign_id, "aggregating temp files");

        let entries = match fs::read_dir(&tmp_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(%e, ?tmp_dir, "failed to read temp dir");
                return Ok(CampaignResult {
                    runs: total_runs,
                    failures: all_failures,
                });
            }
        };
        let mut collected = 0usize;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            trace!(file = %name, "found matching temp file");
            let Ok(data) = fs::read(entry.path()) else {
                warn!(file = %name, "failed to read temp file");
                continue;
            };
            let Ok(result) = serde_json::from_slice::<WorkerResult>(&data) else {
                warn!(file = %name, "failed to parse temp file");
                continue;
            };
            total_runs += result.runs;
            all_failures.extend(result.failures);
            collected += 1;
        }
        info!(
            collected,
            total_runs,
            failures = all_failures.len(),
            "aggregation complete"
        );

        Ok(CampaignResult {
            runs: total_runs,
            failures: all_failures,
        })
    }

    /// Convert a worker count into a LibAFL `Cores` mask.
    fn workers_to_cores(workers: usize) -> Result<libafl_bolts::core_affinity::Cores> {
        let ids = libafl_bolts::core_affinity::get_core_ids()
            .map(|v| v.len())
            .unwrap_or(1);
        let count = workers.min(ids);

        let mask = if count >= ids {
            "all".into()
        } else {
            (0..count)
                .map(|i| format!("{i}"))
                .collect::<Vec<String>>()
                .join(",")
        };

        libafl_bolts::core_affinity::Cores::from_cmdline(&mask)
            .with_context(|| format!("failed to parse core mask '{mask}'"))
    }
}

/// Build seed inputs from the contract ABI.
pub fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<input::CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = input::Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(input::CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<input::Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| input::Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = input::CallSequenceInput::new();
        combined.calls = action_calls.clone();
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Vec::new());
        let mut permutations = Vec::new();
        while let Some(prefix) = queue.pop_front() {
            if prefix.len() == n {
                permutations.push(prefix);
                continue;
            }
            for (idx, _call) in action_calls.iter().enumerate() {
                let already_in_prefix = prefix.contains(&idx);
                if !already_in_prefix {
                    let mut next = prefix.to_vec();
                    next.push(idx);
                    queue.push_back(next);
                }
            }
        }
        for perm in permutations {
            let mut seq = input::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{Campaign, CampaignConfig};
    use crate::contract;

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/ConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ComplexConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/ComplexConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ComplexConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/SetupRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/SetupRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        assert!(
            !artifact.properties.is_empty(),
            "property_caught() should be discovered as a property"
        );

        let mut config = CampaignConfig::default();
        config.workers = 1;
        config.max_runs = 10_000;
        let campaign = Campaign::for_target(Path::new("src/L1SimpleKnob.sol"))
            .with_project(Path::new("fixtures/challenges"))
            .with_config(config)
            .build()
            .unwrap();
        let result = campaign.run().unwrap();

        assert!(
            !result.failures.is_empty(),
            "raptor should find at least one property failure (dragon caught)"
        );
    }
}
