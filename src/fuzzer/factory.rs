//! Fuzzer factory: owns the chain and creates per-thread [`Fuzzer`] instances.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::{Address, Selector};
use anyhow::Result;
use tracing::{info, instrument};

use crate::evm;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::SharedCorpus;
use crate::fuzzer::corpus::{Call, Item};
use crate::fuzzer::engine;
use crate::fuzzer::metrics::SharedMetrics;
use crate::target;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct FuzzerResult {
    pub runs: u64,
    pub failures: Vec<Crash>,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// A single crash (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Crash {
    pub function_name: String,
    pub selector: Selector,
    pub call_sequence: Vec<Call>,
}

/// Factory that owns the base chain state and spawns [`Fuzzer`] instances.
///
/// The factory holds the post-deployment, post-setup chain snapshot.
/// Each [`Fuzzer`] receives an independent clone of this snapshot so
/// sequences execute against isolated state.
#[derive(Debug, Clone)]
pub struct Factory {
    chain: evm::Chain,
    contract: Arc<target::Contract>,
    deployed_address: Address,
    config: Config,
    caller: Address,
    corpus: SharedCorpus,
    metrics: SharedMetrics,
}

impl Factory {
    /// Create a new factory.
    pub fn new(
        chain: evm::Chain,
        contract: target::Contract,
        deployed_address: Address,
        config: Config,
        corpus: SharedCorpus,
    ) -> Self {
        Self {
            chain,
            contract: Arc::new(contract),
            deployed_address,
            config,
            caller: evm::chain::DEFAULT_DEPLOYER,
            corpus,
            metrics: SharedMetrics::new(),
        }
    }

    /// Set the default caller address used for fuzz transactions.
    pub fn with_caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Provide shared metrics.
    pub fn with_metrics(mut self, metrics: SharedMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Access the shared corpus.
    pub fn corpus(&self) -> &SharedCorpus {
        &self.corpus
    }

    /// Access the shared metrics.
    pub fn metrics(&self) -> &SharedMetrics {
        &self.metrics
    }

    /// Create a new [`Fuzzer`] for the given thread id.
    pub fn create(&self, fuzzer_id: usize) -> Fuzzer {
        Fuzzer {
            chain: self.chain.clone(),
            contract: Arc::clone(&self.contract),
            deployed_address: self.deployed_address,
            config: Config {
                seed: self.config.seed.wrapping_add(fuzzer_id as u64),
                ..self.config
            },
            caller: self.caller,
            corpus: self.corpus.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

/// Per-thread fuzzer that executes call sequences and reports results.
///
/// Created by [`Factory::create`] and run via [`Fuzzer::run`].
pub struct Fuzzer {
    chain: evm::Chain,
    contract: Arc<target::Contract>,
    deployed_address: Address,
    config: Config,
    caller: Address,
    corpus: SharedCorpus,
    metrics: SharedMetrics,
}

impl std::fmt::Debug for Fuzzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fuzzer")
            .field("chain", &self.chain)
            .field("contract", &self.contract)
            .field("deployed_address", &self.deployed_address)
            .field("config", &self.config)
            .field("caller", &self.caller)
            .field("corpus", &self.corpus)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl Fuzzer {
    /// Run the fuzzer for up to `max_runs` iterations.
    ///
    /// The fuzzer loop uses the shared corpus for mutation and the shared
    /// metrics for counters. It stops early if `timeout` is reached.
    #[instrument(skip(self), fields(max_runs))]
    pub fn run(&mut self, max_runs: u64, timeout: Option<Duration>) -> Result<FuzzerResult> {
        let start = Instant::now();
        let mut rng = fastrand::Rng::with_seed(self.config.seed);
        let mut local_failures = Vec::new();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        for _ in 0..max_runs {
            if let Some(t) = timeout
                && start.elapsed() > t
            {
                break;
            }

            if let Some(snapshot) = self.metrics.maybe_print() {
                info!(
                    elapsed = ?snapshot.elapsed,
                    runs = snapshot.runs,
                    calls = snapshot.calls,
                    gas = snapshot.gas,
                    failures = snapshot.failures,
                    "fuzz metrics",
                );
            }

            let item = self.corpus.take(&mut rng);
            let calls = item.calls;

            let outcome = engine::execute_sequence(
                &self.chain,
                &self.contract,
                self.deployed_address,
                self.caller,
                &calls,
            )?;

            total_calls += outcome.total_calls;
            total_gas += outcome.total_gas;
            self.metrics.record(outcome.total_calls, outcome.total_gas);

            if outcome.all_ok {
                // checkrs: allow(clone_in_loops)
                let _ = self.corpus.add(Item::from(calls.clone()));
            }

            if let Some(crash_info) = outcome.crash {
                local_failures.push(Crash {
                    function_name: crash_info.name,
                    selector: crash_info.selector,
                    call_sequence: calls,
                });
                self.metrics.record_failure();
            }

            runs += 1;
        }

        info!(runs, "fuzzer run finished");
        Ok(FuzzerResult {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}

/// Format a crash's call sequence as a flat, Medusa-style log.
pub fn format_failure(
    contract: &target::Contract,
    failure: &Crash,
    sender: revm::primitives::Address,
) -> String {
    let mut lines = Vec::new();
    for (i, call) in failure.call_sequence.iter().enumerate() {
        let n = i + 1;

        let block = n as u64;
        let time = n as u64;

        let func_name = call.function.name.as_str();
        let args = match &call.args {
            alloy_dyn_abi::DynSolValue::Tuple(v) if v.is_empty() => "()".into(),
            alloy_dyn_abi::DynSolValue::Tuple(v) => {
                let args_str = v
                    .iter()
                    .map(format_dyn_value)
                    .collect::<Vec<String>>()
                    .join(", ");
                format!("({})", args_str)
            }
            other => format_dyn_value(other),
        };

        lines.push(format!(
            "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?})",
            n,
            contract.artifact_id.name,
            func_name,
            args,
            block,
            time,
            u64::MAX,
            sender,
        ));
    }
    lines.join("\n")
}

fn format_dyn_value(v: &alloy_dyn_abi::DynSolValue) -> String {
    match v {
        alloy_dyn_abi::DynSolValue::Bool(b) => format!("{}", b),
        alloy_dyn_abi::DynSolValue::Int(i, _) => format!("{}", i),
        alloy_dyn_abi::DynSolValue::Uint(u, _) => format!("{}", u),
        alloy_dyn_abi::DynSolValue::Address(a) => format!("{:?}", a),
        alloy_dyn_abi::DynSolValue::String(s) => format!("\"{}\"", s),
        alloy_dyn_abi::DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        alloy_dyn_abi::DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}
