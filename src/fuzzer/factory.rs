//! Fuzzer factory: owns the chain and creates per-thread [`Fuzzer`] instances.

use std::sync::Arc;

use alloy_primitives::{Address, Selector};

use crate::evm;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::Call;
use crate::fuzzer::corpus::SharedCorpus;
use crate::fuzzer::fuzzer::Fuzzer;
use crate::fuzzer::metrics::Shared as MetricsShared;
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
    metrics: MetricsShared,
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
            metrics: MetricsShared::new(),
        }
    }

    /// Set the default caller address used for fuzz transactions.
    pub fn with_caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Provide shared metrics.
    pub fn with_metrics(mut self, metrics: MetricsShared) -> Self {
        self.metrics = metrics;
        self
    }

    /// Access the shared corpus.
    pub fn corpus(&self) -> &SharedCorpus {
        &self.corpus
    }

    /// Access the shared metrics.
    pub fn metrics(&self) -> &MetricsShared {
        &self.metrics
    }

    /// Create a new [`Fuzzer`] for the given thread id.
    pub fn create(&self, fuzzer_id: usize) -> Fuzzer {
        let seed = self.config.seed.wrapping_add(fuzzer_id as u64);
        Fuzzer::new(
            self.chain.clone(),
            Arc::clone(&self.contract),
            self.deployed_address,
            Config {
                seed,
                ..self.config
            },
            self.caller,
            self.corpus.clone(),
            self.metrics.clone(),
            fastrand::Rng::with_seed(seed),
        )
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
