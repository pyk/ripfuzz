//! Maximization campaign: maximize a single `max_*` function's return value.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, ensure};
use tracing::{error, info};

use crate::campaigns::{CampaignKind, CampaignSession, split_runs, wait_for_workers};
use crate::corpus::{CorpusConfig, Item, SharedFailedCorpusItem};
use crate::evm::Transaction;
use crate::formatter;
use crate::fuzzer::{FailedAssertion, SharedFailedAssertions, SharedMetrics};
use crate::max::{
    MaxBestItem, MaxFuzzer, MaxFuzzerConfig, MaxFuzzerCorpus, MaxObjective, MaxResult, MaxShrinker,
    MaxShrinkerConfig, MaxShrinkerCorpus,
};
use crate::shrinker::{Shrinker, ShrinkerConfig};

/// Maximization campaign.
pub struct MaxCampaign {
    session: CampaignSession,
    objective: MaxObjective,
}

impl MaxCampaign {
    /// Create a max campaign from a prepared session.
    pub fn new(session: CampaignSession) -> Result<Self> {
        ensure!(
            session.kind == CampaignKind::Max,
            "cannot run a max campaign on an invariant-mode harness"
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
        let shared_failed_assertions = SharedFailedAssertions::for_campaign(
            self.session.args.max_failures,
            self.session.args.fail_on_revert,
        );
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        let fuzzer_corpus = MaxFuzzerCorpus::new(self.session.corpus.clone());

        let fuzzers = self.session.args.threads;
        let timeout = self
            .session
            .args
            .timeout_secs
            .map(std::time::Duration::from_secs);

        let initial_config = MaxFuzzerConfig::new()
            .chain(self.session.chain.clone())
            .target_address(self.session.deployed_address)
            .shared_corpus(fuzzer_corpus.clone())
            .shared_coverage(self.session.shared_coverage.clone())
            .shared_metrics(shared_metrics.clone())
            .shared_failed_assertions(shared_failed_assertions.clone())
            .shutdown_signal(shutdown_signal.clone())
            .caller(self.session.args.deployer_address)
            .objective(self.objective.clone())
            .gas_limit(self.session.args.gas_limit)
            .timeout(timeout)
            .fail_on_revert(self.session.args.fail_on_revert);

        let mut handles = Vec::with_capacity(fuzzers);
        for (fuzzer_id, local_max_runs) in
            split_runs(self.session.args.max_runs, fuzzers).enumerate()
        {
            let seed = self.session.campaign_seed.wrapping_add(fuzzer_id as u64);
            // checkrs: allow(clone_in_loops)
            let mut config = initial_config.clone();
            config.max_runs = local_max_runs;
            config.seed = seed;

            let fuzzer = MaxFuzzer::new(config);
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

        info!("[+] fuzzed {contract_name} with {fuzzers} threads");
        let function_metrics = shared_metrics.function_metrics();
        let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
        info!("{stats}");

        let mut shrunk_failures = Vec::new();
        let failed_assertions = shared_failed_assertions.items();
        if !failed_assertions.is_empty() {
            shrunk_failures = self.shrink_failures(&failed_assertions)?;
        }

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

        // Re-run each shrunk failed assertion with the chain tracer enabled.
        for (assertion_number, shrunk_item) in &shrunk_failures {
            // checkrs: allow(clone_in_loops)
            let mut trace_chain = self.session.chain.clone();
            trace_chain.set_trace(true);

            let objective_transaction = self.objective.transaction(
                self.session.deployed_address,
                self.session.args.deployer_address,
                self.session.args.gas_limit,
            );
            let transactions: Vec<Transaction> = shrunk_item
                .calls
                .iter()
                .map(|call| call.into_transaction(self.session.deployed_address))
                .collect();
            let mut interleaved = Vec::with_capacity(transactions.len() * 2);
            for transaction in transactions {
                interleaved.push(transaction);
                // checkrs: allow(clone_in_loops)
                interleaved.push(objective_transaction.clone());
            }

            let exec = trace_chain.exec(&interleaved)?;

            if let Some(trace) = exec.trace {
                let trace_name = if shrunk_failures.len() == 1 {
                    "trace-max-fail.log".to_owned()
                } else {
                    format!("trace-max-fail-{assertion_number}.log")
                };
                info!("[*] writing trace {assertion_number} ...");
                match self.session.write_trace(&trace, &trace_name) {
                    Ok(trace_file) => {
                        info!("[+] trace {assertion_number}: {}", trace_file.display());
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
    fn shrink_max(&mut self, best: MaxBestItem) -> Result<MaxResult> {
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

        let shrink_corpus =
            MaxShrinkerCorpus::new(best.item, best.value, shrink_config, session.corpus.clone());

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
            let shrinker_config = MaxShrinkerConfig::new()
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
            let shrinker = MaxShrinker::new(shrinker_config);
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

        Ok(MaxResult {
            objective,
            value: shrunk.value,
            item: shrunk.item,
        })
    }

    /// Shrink each failed assertion and return the shrunken items.
    ///
    /// Candidates execute with the max objective interleaved after every call
    /// (stride-2 layout), so objective-call reverts are preserved while
    /// shrinking.
    fn shrink_failures(
        &mut self,
        failed_assertions: &[FailedAssertion],
    ) -> Result<Vec<(usize, Item)>> {
        let session = &mut self.session;
        let objective = self.objective.clone();

        let all_function_signatures: Vec<String> = session
            .harness_contract
            .handler_functions
            .iter()
            .chain(session.harness_contract.max_functions.iter())
            .map(|f| f.signature())
            .collect();

        let failed_corpus_config = CorpusConfig::new(PathBuf::new())
            .handler_functions(session.harness_contract.handler_functions.clone())
            .max_calls(session.args.max_calls)
            .literals(session.literals.clone());

        let objective_transaction = objective.transaction(
            session.deployed_address,
            session.args.deployer_address,
            session.args.gas_limit,
        );

        let shrink_threads = session.args.shrink_threads.unwrap_or(session.args.threads);
        let shrink_timeout = session
            .args
            .shrink_timeout_secs
            .map(std::time::Duration::from_secs);

        let assertion_word = if failed_assertions.len() == 1 {
            "assertion"
        } else {
            "assertions"
        };
        error!(
            "[!] found {} distinct failed {assertion_word}",
            failed_assertions.len()
        );

        let runs_per_assertion = (session.args.shrink_runs / failed_assertions.len() as u64).max(1);
        let mut shrunk_assertions = Vec::with_capacity(failed_assertions.len());

        for (assertion_index, assertion) in failed_assertions.iter().enumerate() {
            let assertion_number = assertion_index + 1;
            let initial_calls = assertion.item.calls.len();

            let shared_failed_item = SharedFailedCorpusItem::new(
                // checkrs: allow(clone_in_loops)
                Item::from(assertion.item.calls.clone()),
                // checkrs: allow(clone_in_loops)
                failed_corpus_config.clone(),
            );

            let shrinker_shutdown = Arc::new(AtomicBool::new(false));
            // checkrs: allow(clone_in_loops)
            let shrinker_metrics = SharedMetrics::new(all_function_signatures.clone());

            let mut shrinker_handles = Vec::with_capacity(shrink_threads);
            for (shrinker_id, local_max_runs) in
                split_runs(runs_per_assertion, shrink_threads).enumerate()
            {
                let seed = session
                    .campaign_seed
                    .wrapping_add(shrinker_id as u64)
                    .wrapping_add(1000 + assertion_index as u64 * 1000);
                // checkrs: allow(clone_in_loops)
                let shrinker_chain = session.chain.clone();
                // checkrs: allow(clone_in_loops)
                let shrinker_shared_item = shared_failed_item.clone();
                // checkrs: allow(clone_in_loops)
                let shrinker_shutdown = shrinker_shutdown.clone();
                // checkrs: allow(clone_in_loops)
                let shrinker_objective_transaction = objective_transaction.clone();
                let shrinker_config = ShrinkerConfig::new()
                    .chain(shrinker_chain)
                    .target_address(session.deployed_address)
                    .shared_failed_item(shrinker_shared_item)
                    .shutdown_signal(shrinker_shutdown)
                    .max_runs(local_max_runs)
                    .timeout(shrink_timeout)
                    .seed(seed)
                    // checkrs: allow(clone_in_loops)
                    .shared_metrics(shrinker_metrics.clone())
                    .fail_on_revert(session.args.fail_on_revert)
                    .objective_transaction(Some(shrinker_objective_transaction));
                let shrinker = Shrinker::new(shrinker_config);
                let handle = std::thread::spawn(move || shrinker.run());
                shrinker_handles.push(handle);
            }

            info!(
                "[*] shrinking assertion {assertion_number}/{} from {} calls with {} threads",
                failed_assertions.len(),
                formatter::num(initial_calls as u64),
                formatter::num(shrink_threads as u64)
            );
            wait_for_workers(&shrinker_handles, || {
                if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                    let current_calls = shared_failed_item.item().calls.len();
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
                        error!(%e, "shrinker failed");
                    }
                    Err(e) => {
                        error!(?e, "shrinker panicked");
                    }
                }
            }

            let shrunk_item = shared_failed_item.item();
            let shrunk_calls = shrunk_item.calls.len();
            info!(
                "[+] shrank assertion {assertion_number}/{} from {} to {} calls with {} threads",
                failed_assertions.len(),
                formatter::num(initial_calls as u64),
                formatter::num(shrunk_calls as u64),
                formatter::num(shrink_threads as u64)
            );
            let snapshot = shrinker_metrics.aggregate();
            info!(
                "{}",
                formatter::shrinker_summary(&snapshot, initial_calls, shrunk_calls)
            );
            shrunk_assertions.push((assertion_number, shrunk_item));
        }

        Ok(shrunk_assertions)
    }
}
