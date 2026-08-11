//! Maxxing campaign: maximize a single `max_*` function's return value.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, ensure};
use tracing::{error, info};

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
            config.max_runs = local_max_runs;
            config.seed = seed;

            let fuzzer = MaxxingFuzzer::new(config);
            let handle = std::thread::spawn(move || fuzzer.run());
            handles.push((fuzzer_id, handle));
        }

        let contract_name = self.session.contract_name();
        info!("[*] max fuzzing {contract_name} with {fuzzers} threads");

        let stats_ctx = formatter::CampaignStats::new(
            &self.session.shared_coverage,
            &self.session.corpus,
            &self.session.harness_contract.handler_functions,
            &[],
            &self.session.harness_contract.max_functions,
        );

        wait_for_workers(handles.iter().map(|(_, handle)| handle), || {
            if let Some(snapshot) = shared_metrics.try_snapshot() {
                info!("[~] {}", stats_ctx.progress(&snapshot));
            }
            Ok(())
        })?;

        for (fuzzer_id, handle) in handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error!(fuzzer_id, %e, "max fuzzer failed");
                }
                Err(e) => {
                    error!(fuzzer_id, ?e, "max fuzzer panicked");
                }
            }
        }

        // Stop-on-revert: dump the whole trace into the log (file and
        // stderr) and skip the shrinking phase entirely.
        if let Some(event) = shared_stop_event.get() {
            error!("[!] a transaction reverted; stopping the campaign (--stop-on-revert)");
            if let Err(e) = self.session.dump_trace_sequence(&event.transactions) {
                error!("[!] failed to dump the revert trace: {e:#}");
            }
            if let Err(e) = self.session.write_coverage_report() {
                error!("[!] failed to generate coverage reports: {e:#}");
            }
            info!("[*] ripfuzz out. see ya");
            return Ok(());
        }

        info!("[+] fuzzed {contract_name} with {fuzzers} threads");
        let function_metrics = shared_metrics.function_metrics();
        let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
        info!("{stats}");

        let mut results = Vec::new();
        if let Some(best) = fuzzer_corpus.best_item() {
            results.push(self.shrink_max(best)?);
        }

        if results.is_empty() {
            error!("[!] no max value improved above 0");
        } else {
            for result in &results {
                info!(
                    "    max {} = {}",
                    result.objective.function.name, result.value
                );
                info!("{}", result.format_call_sequence());
            }
        }

        // Re-run each shrunk item with the chain tracer enabled.
        for (index, result) in results.iter().enumerate() {
            // checkrs: allow(clone_in_loops)
            let mut trace_chain = self.session.chain.clone();
            trace_chain.set_trace(true);

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

            let exec = trace_chain.exec(&transactions)?;

            if let Some(trace) = exec.trace {
                info!("[*] writing max trace {} ...", index + 1);
                match self
                    .session
                    .write_trace(&trace, &format!("trace-max-{}.log", index + 1))
                {
                    Ok(trace_file) => {
                        info!("[+] max trace {}: {}", index + 1, trace_file.display());
                    }
                    Err(e) => {
                        error!("[!] writing trace file failed: {e:#}");
                        return Err(e);
                    }
                }
            }
        }

        if let Err(e) = self.session.write_coverage_report() {
            error!("[!] failed to generate coverage reports: {e:#}");
        }

        info!("[*] ripfuzz out. see ya");
        Ok(())
    }

    /// Shrink the best max result and report it.
    fn shrink_max(&mut self, best: MaxBestItem) -> Result<MaxxingResult> {
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
            "[*] shrinking max {} from {} calls with {} threads",
            objective.function.name,
            formatter::num(initial_calls as u64),
            formatter::num(shrink_threads as u64)
        );
        wait_for_workers(&shrinker_handles, || {
            if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                let current_calls = shrink_corpus.item().item.calls.len();
                info!(
                    "[~] {}",
                    formatter::shrinker_progress(&snapshot, initial_calls, current_calls)
                );
            }
            Ok(())
        })?;

        for handle in shrinker_handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error!(%e, "max shrinker failed");
                }
                Err(e) => {
                    error!(?e, "max shrinker panicked");
                }
            }
        }

        let shrunk = shrink_corpus.item();
        let shrunk_calls = shrunk.item.calls.len();
        info!(
            "[+] shrank max {} from {} to {} calls with {} threads",
            objective.function.name,
            formatter::num(initial_calls as u64),
            formatter::num(shrunk_calls as u64),
            formatter::num(shrink_threads as u64)
        );

        Ok(MaxxingResult {
            objective,
            value: shrunk.value,
            item: shrunk.item,
        })
    }
}
