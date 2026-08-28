//! Maxxing campaign: maximize a single `max_*` function's return value.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use alloy_primitives::U256;
use anyhow::{Context, Result, ensure};
use tracing::{error, info, info_span, warn};

use crate::campaigns::{CampaignKind, CampaignSession, split_runs, wait_for_workers};
use crate::corpus::CorpusConfig;
use crate::evm::Transaction;
use crate::formatter;
use crate::fuzzers::{
    MaxBestItem, MaxObjective, MaxxingFuzzer, MaxxingFuzzerConfig, MaxxingFuzzerCorpus,
    SharedMetrics, SharedStopEvent,
};
use crate::shrinkers::{
    MaxxingResult, MaxxingShrinker, MaxxingShrinkerConfig, MaxxingShrinkerCorpus,
};

/// Maxxing campaign.
pub struct MaxxingCampaign {
    session: CampaignSession,
    objective: MaxObjective,
}

impl MaxxingCampaign {
    /// Create a maxxing campaign from a prepared session.
    pub fn new(session: CampaignSession) -> Result<Self> {
        ensure!(
            session.kind == CampaignKind::Maxxing,
            "cannot run a maxxing campaign on an invariant-mode harness"
        );
        let objective = MaxObjective::new(
            session
                .harness_contract
                .max_functions
                .first()
                .context("max mode requires exactly one `max_*` function")?
                .clone(),
        );
        Ok(Self { session, objective })
    }

    /// Run the campaign: fuzz, shrink the best result, trace, and report.
    pub fn run(mut self) -> Result<()> {
        let all_function_signatures: Vec<String> = self
            .session
            .harness_contract
            .handler_functions
            .iter()
            .chain(self.session.harness_contract.max_functions.iter())
            .map(|f| f.signature())
            .collect();
        let shared_metrics = SharedMetrics::new(all_function_signatures.clone());
        let shared_stop_event = SharedStopEvent::new();
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        let fuzzer_corpus = MaxxingFuzzerCorpus::new(self.session.corpus.clone());
        let baseline = {
            let mut baseline_chain = self.session.chain.clone();
            let tx = self.objective.transaction(
                self.session.deployed_address,
                self.session.args.deployer_address,
                self.session.args.gas_limit,
            );
            let exec = baseline_chain
                .exec(&[tx])
                .context("failed to call max_* for baseline after setup")?;
            let result = exec
                .results
                .first()
                .context("missing max_* result for baseline")?;
            self.objective.decode(result).with_context(|| {
                format!(
                    "max_* {} did not return uint256 after setup (reverted or empty)",
                    self.objective.function.signature()
                )
            })?
        };
        fuzzer_corpus.set_baseline(baseline);
        info!(baseline = %baseline, "maxxing baseline after setup");

        let fuzzers = self.session.args.threads;
        let timeout = self
            .session
            .args
            .timeout_secs
            .map(std::time::Duration::from_secs);

        let initial_config = MaxxingFuzzerConfig::new()
            .chain(self.session.chain.clone())
            .target_address(self.session.deployed_address)
            .shared_corpus(fuzzer_corpus.clone())
            .shared_coverage(self.session.shared_coverage.clone())
            .shared_metrics(shared_metrics.clone())
            .shared_stop_event(shared_stop_event.clone())
            .shutdown_signal(shutdown_signal.clone())
            .caller(self.session.args.deployer_address)
            .objective(self.objective.clone())
            .gas_limit(self.session.args.gas_limit)
            .timeout(timeout)
            .stop_on_revert(self.session.args.stop_on_revert);

        let mut handles = Vec::with_capacity(fuzzers);
        for (fuzzer_id, local_max_runs) in
            split_runs(self.session.args.max_runs, fuzzers).enumerate()
        {
            let seed = self.session.campaign_seed.wrapping_add(fuzzer_id as u64);
            // checkrs: allow(clone_in_loops)
            let mut config = initial_config.clone();
            config.fuzzer_id = fuzzer_id;
            config.max_runs = local_max_runs;
            config.seed = seed;

            let fuzzer = MaxxingFuzzer::new(config);
            let handle = std::thread::spawn(move || fuzzer.run());
            handles.push((fuzzer_id, handle));
        }

        let span = info_span!("maxxing", threads = fuzzers);
        let _guard = span.enter();
        info!("started");

        let stats_ctx = formatter::CampaignStats::new(
            &self.session.shared_coverage,
            &self.session.corpus,
            &self.session.harness_contract.handler_functions,
            &[],
            &self.session.harness_contract.max_functions,
        );

        wait_for_workers(handles.iter().map(|(_, handle)| handle), || {
            if let Some(snapshot) = shared_metrics.try_snapshot() {
                let value = fuzzer_corpus.best_value().unwrap_or(U256::ZERO);
                let function_metrics = shared_metrics.function_metrics();
                stats_ctx.log_maxxing_summary(
                    &snapshot,
                    value,
                    self.session.chain.rpc_stats(),
                    &function_metrics,
                    "progress",
                );
            }
            Ok(())
        })?;

        let mut failures: Vec<anyhow::Error> = Vec::new();
        for (fuzzer_id, handle) in handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error!(fuzzer_id, "max fuzzer failed: {e:#}");
                    failures.push(e);
                }
                Err(e) => {
                    error!(fuzzer_id, ?e, "max fuzzer panicked");
                    failures.push(anyhow::anyhow!("max fuzzer {fuzzer_id} panicked: {e:?}"));
                }
            }
        }
        if !failures.is_empty() {
            let count = failures.len();
            let first = failures.remove(0);
            return Err(first).with_context(|| format!("{count} max fuzzer threads failed"));
        }

        // Stop-on-revert: log a single multi-line error message carrying the
        // compact trace, write the full trace to a trace file, and fail the
        // campaign immediately.
        if let Some(event) = shared_stop_event.get() {
            match self
                .session
                .trace_sequence_to_file(&event.transactions, "fulltrace.log")
            {
                Ok(report) => {
                    error!("a transaction reverted.\n\n{}", report.compact);
                    let log = self
                        .session
                        .log_file
                        .as_ref()
                        .map(|path| format!("\nlog: {}", path.display()))
                        .unwrap_or_default();
                    return Err(anyhow::anyhow!(
                        "campaign stopped by --stop-on-revert\nfulltrace: {}{}",
                        report.file.display(),
                        log
                    ));
                }
                Err(e) => {
                    error!("failed to dump the revert trace: {e:#}");
                }
            }
            return Err(anyhow::anyhow!("campaign stopped by --stop-on-revert"));
        }

        let function_metrics = shared_metrics.function_metrics();
        let value = fuzzer_corpus.best_value().unwrap_or(U256::ZERO);
        stats_ctx.log_maxxing_summary(
            &shared_metrics.aggregate(),
            value,
            self.session.chain.rpc_stats(),
            &function_metrics,
            "finished",
        );
        for stat in stats_ctx.function_stats(&function_metrics) {
            stat.log();
        }
        drop(_guard);
        drop(span);

        let shrink_threads = self
            .session
            .args
            .shrink_threads
            .unwrap_or(self.session.args.threads);
        let shrink_span = info_span!("shrink", threads = shrink_threads);
        let _shrink_guard = shrink_span.enter();

        let mut results = Vec::new();
        if let Some(best) = fuzzer_corpus.best_item() {
            results.push(self.shrink_max(best, baseline)?);
        }

        if results.is_empty() {
            warn!(
                objective = %self.objective.function.name,
                "no sequence improved the max value (best stayed at 0)",
            );
        } else {
            for result in &results {
                info!(
                    max = %result.objective.function.name,
                    value = %result.value,
                    "maximum value",
                );
            }
        }
        drop(_shrink_guard);

        let trace_span = info_span!("trace");
        let _trace_guard = trace_span.enter();

        // Re-run each shrunk item with the chain tracer enabled, dumping the
        // compact trace into the log and the full trace to a trace file.
        for (index, result) in results.iter().enumerate() {
            let mut transactions: Vec<Transaction> = result
                .item
                .calls
                .iter()
                .map(|call| call.into_transaction(self.session.deployed_address))
                .collect();
            transactions.push(result.objective.transaction(
                self.session.deployed_address,
                self.session.args.deployer_address,
                self.session.args.gas_limit,
            ));
            if let Some(summary) = self.session.harness_contract.summary_transaction(
                self.session.deployed_address,
                self.session.args.deployer_address,
            ) {
                transactions.push(summary);
            }

            let trace_name = format!("fulltrace-max-{}.log", index + 1);
            match self
                .session
                .trace_sequence_to_file(&transactions, &trace_name)
            {
                Ok(report) => {
                    if !report.logs.is_empty() {
                        info!("{}", report.logs);
                    }
                    info!("{}", report.file.display());
                }
                Err(e) => {
                    error!("writing trace file failed: {e:#}");
                    return Err(e);
                }
            }
        }
        drop(_trace_guard);

        if let Some(log_file) = &self.session.log_file {
            let log_span = info_span!("log");
            let _log_guard = log_span.enter();
            info!("{}", log_file.display());
        }

        if let Err(e) = self.session.write_coverage_report() {
            error!("failed to generate coverage reports: {e:#}");
        }

        Ok(())
    }

    /// Shrink the best max result and report it.
    fn shrink_max(&mut self, best: MaxBestItem, baseline: U256) -> Result<MaxxingResult> {
        let objective = self.objective.clone();
        let session = &mut self.session;

        let all_function_signatures: Vec<String> = session
            .harness_contract
            .handler_functions
            .iter()
            .chain(session.harness_contract.max_functions.iter())
            .map(|f| f.signature())
            .collect();

        let shrink_config = CorpusConfig::new(PathBuf::new())
            .handler_functions(session.harness_contract.handler_functions.clone())
            .max_calls(session.args.max_calls)
            .literals(session.literals.clone());
        let shrink_threads = session.args.shrink_threads.unwrap_or(session.args.threads);
        let shrink_timeout = session
            .args
            .shrink_timeout_secs
            .map(std::time::Duration::from_secs);

        let shrink_corpus = MaxxingShrinkerCorpus::new(
            best.item,
            best.value,
            baseline,
            shrink_config,
            session.corpus.clone(),
        );

        let runs_per_result = session.args.shrink_runs.max(1);
        let shrinker_shutdown = Arc::new(AtomicBool::new(false));
        // checkrs: allow(clone_in_loops)
        let shrinker_metrics = SharedMetrics::new(all_function_signatures);

        let mut shrinker_handles = Vec::with_capacity(shrink_threads);
        for (shrinker_id, local_max_runs) in split_runs(runs_per_result, shrink_threads).enumerate()
        {
            let seed = session
                .campaign_seed
                .wrapping_add(shrinker_id as u64)
                .wrapping_add(2000);
            // checkrs: allow(clone_in_loops)
            let shrinker_chain = session.chain.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_corpus = shrink_corpus.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_shutdown = shrinker_shutdown.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_objective = objective.clone();
            let shrinker_config = MaxxingShrinkerConfig::new()
                .chain(shrinker_chain)
                .target_address(session.deployed_address)
                .shared_corpus(shrinker_corpus)
                .shutdown_signal(shrinker_shutdown)
                .objective(shrinker_objective)
                .max_runs(local_max_runs)
                .timeout(shrink_timeout)
                .seed(seed)
                // checkrs: allow(clone_in_loops)
                .shared_metrics(shrinker_metrics.clone())
                .gas_limit(session.args.gas_limit)
                .caller(session.args.deployer_address);
            let shrinker = MaxxingShrinker::new(shrinker_config);
            let handle = std::thread::spawn(move || shrinker.run());
            shrinker_handles.push(handle);
        }

        let initial_calls = shrink_corpus.item().item.calls.len();
        info!(
            max = %objective.function.name,
            initial_calls = %formatter::num(initial_calls as u64),
            "shrinking max",
        );
        wait_for_workers(&shrinker_handles, || {
            if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                let current_calls = shrink_corpus.item().item.calls.len();
                formatter::log_shrinker_progress(&snapshot, initial_calls, current_calls);
            }
            Ok(())
        })?;

        let mut failures: Vec<anyhow::Error> = Vec::new();
        for handle in shrinker_handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error!("max shrinker failed: {e:#}");
                    failures.push(e);
                }
                Err(e) => {
                    error!(?e, "max shrinker panicked");
                    failures.push(anyhow::anyhow!("max shrinker panicked: {e:?}"));
                }
            }
        }
        if !failures.is_empty() {
            let count = failures.len();
            let first = failures.remove(0);
            return Err(first).with_context(|| format!("{count} max shrinker threads failed"));
        }

        let shrunk = shrink_corpus.item();
        let shrunk_calls = shrunk.item.calls.len();
        info!(
            max = %objective.function.name,
            initial_calls = %formatter::num(initial_calls as u64),
            final_calls = %formatter::num(shrunk_calls as u64),
            "shrank max",
        );

        Ok(MaxxingResult {
            objective,
            value: shrunk.value,
            item: shrunk.item,
        })
    }
}
