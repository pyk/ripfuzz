//! Invariant campaign: validate invariants across generated call sequences
//! and shrink every distinct failed assertion.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, ensure};
use tracing::{error, info, info_span, warn};

use crate::campaigns::{CampaignKind, CampaignSession, split_runs, wait_for_workers};
use crate::corpus::{Call, CorpusConfig, Item, SharedFailedCorpusItem};
use crate::evm::Transaction;
use crate::formatter;
use crate::fuzzers::{
    FailedAssertion, InvariantFuzzer, InvariantFuzzerConfig, SharedFailedAssertions, SharedMetrics,
    SharedStopEvent,
};
use crate::shrinkers::{InvariantShrinker, InvariantShrinkerConfig};

/// Invariant campaign.
pub struct InvariantCampaign {
    session: CampaignSession,
}

impl InvariantCampaign {
    /// Create an invariant campaign from a prepared session.
    pub fn new(session: CampaignSession) -> Result<Self> {
        ensure!(
            session.kind == CampaignKind::Invariant,
            "cannot run an invariant campaign on a max-mode harness"
        );
        Ok(Self { session })
    }

    /// Run the campaign: fuzz, shrink failed assertions, trace, and report.
    pub fn run(self) -> Result<()> {
        let InvariantCampaign { mut session } = self;

        let all_function_signatures: Vec<String> = session
            .harness_contract
            .handler_functions
            .iter()
            .chain(session.harness_contract.invariant_functions.iter())
            .map(|f| f.signature())
            .collect();
        let shared_metrics = SharedMetrics::new(all_function_signatures.clone());
        let shared_failed_assertions = SharedFailedAssertions::new(session.args.max_failures);
        let shared_stop_event = SharedStopEvent::new();
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        for failure in std::mem::take(&mut session.replay_failures) {
            shared_failed_assertions.try_add(FailedAssertion {
                transactions: failure.transactions,
                item: failure.item,
                failure_index: failure.failure_index,
                failure_pc: failure.failure_pc,
            });
        }

        let fuzzers = session.args.threads;
        let timeout = session
            .args
            .timeout_secs
            .map(std::time::Duration::from_secs);

        let span = info_span!("fuzz", threads = fuzzers);
        let _guard = span.enter();

        // Print a compact progress line every 3 seconds, then a full stats
        // summary after all fuzzer threads finish.
        let stats_ctx = formatter::CampaignStats::new(
            &session.shared_coverage,
            &session.corpus,
            &session.harness_contract.handler_functions,
            &session.harness_contract.invariant_functions,
            &[],
        );

        if shared_failed_assertions.is_full() {
            info!("corpus replay reached --max-failures; skipping fuzzing");
        } else {
            let initial_config = InvariantFuzzerConfig::new()
                .chain(session.chain.clone())
                .target_address(session.deployed_address)
                .shared_corpus(session.corpus.clone())
                .shared_coverage(session.shared_coverage.clone())
                .shared_metrics(shared_metrics.clone())
                .shared_failed_assertions(shared_failed_assertions.clone())
                .shared_stop_event(shared_stop_event.clone())
                .shutdown_signal(shutdown_signal.clone())
                .invariant_functions(session.harness_contract.invariant_functions.clone())
                .caller(session.args.deployer_address)
                .gas_limit(session.args.gas_limit)
                .timeout(timeout)
                .stop_on_revert(session.args.stop_on_revert);

            let mut handles = Vec::with_capacity(fuzzers);
            for (fuzzer_id, local_max_runs) in
                split_runs(session.args.max_runs, fuzzers).enumerate()
            {
                let seed = session.campaign_seed.wrapping_add(fuzzer_id as u64);
                // checkrs: allow(clone_in_loops)
                let mut config = initial_config.clone();
                config.fuzzer_id = fuzzer_id;
                config.max_runs = local_max_runs;
                config.seed = seed;

                let fuzzer = InvariantFuzzer::new(config);
                let handle = std::thread::spawn(move || fuzzer.run());
                handles.push((fuzzer_id, handle));
            }

            info!("started");

            wait_for_workers(handles.iter().map(|(_, handle)| handle), || {
                if let Some(snapshot) = shared_metrics.try_snapshot() {
                    let function_metrics = shared_metrics.function_metrics();
                    stats_ctx.log_summary(
                        &snapshot,
                        session.chain.rpc_stats(),
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
                        error!(fuzzer_id, "fuzzer failed: {e:#}");
                        failures.push(e);
                    }
                    Err(e) => {
                        error!(fuzzer_id, ?e, "fuzzer panicked");
                        failures.push(anyhow::anyhow!("fuzzer {fuzzer_id} panicked: {e:?}"));
                    }
                }
            }
            if !failures.is_empty() {
                let count = failures.len();
                let first = failures.remove(0);
                return Err(first).with_context(|| format!("{count} fuzzer threads failed"));
            }
        }

        // Stop-on-revert: log a single multi-line error message carrying the
        // compact trace, write the full trace to a trace file, and fail the
        // campaign immediately.
        if let Some(event) = shared_stop_event.get() {
            match session.trace_sequence_to_file(&event.transactions, "fulltrace.log") {
                Ok(report) => {
                    error!("a transaction reverted.\n\n{}", report.compact);
                    let log = session
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

        let failed_assertions = shared_failed_assertions.items();
        if failed_assertions.is_empty() {
            let function_metrics = shared_metrics.function_metrics();
            stats_ctx.log_summary(
                &shared_metrics.aggregate(),
                session.chain.rpc_stats(),
                &function_metrics,
                "finished",
            );
            for stat in stats_ctx.function_stats(&function_metrics) {
                stat.log();
            }

            info!("no failed assertions found!");
            drop(_guard);
            drop(span);

            if let Err(e) = session.write_coverage_report() {
                error!("failed to generate coverage reports: {e:#}");
            }

            return Ok(());
        }

        let shrink_threads = session.args.shrink_threads.unwrap_or(session.args.threads);
        let shrink_timeout = session
            .args
            .shrink_timeout_secs
            .map(std::time::Duration::from_secs);

        let invariant_calls: Vec<Call> = session
            .harness_contract
            .invariant_functions
            .iter()
            // checkrs: allow(clone_in_iterator)
            .map(|func| Call {
                function: func.clone(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![]),
                value: None,
                caller: session.args.deployer_address,
            })
            .collect();

        // Include both handler and invariant functions so the shrinker can
        // generate replacement calls for any position in the sequence.
        let all_functions: Vec<alloy_json_abi::Function> = session
            .harness_contract
            .handler_functions
            .iter()
            .chain(session.harness_contract.invariant_functions.iter())
            .cloned()
            .collect();

        let failed_corpus_config = CorpusConfig::new(PathBuf::new())
            .handler_functions(all_functions)
            .max_calls(session.args.max_calls)
            .literals(session.literals.clone());

        let function_metrics = shared_metrics.function_metrics();
        stats_ctx.log_summary(
            &shared_metrics.aggregate(),
            session.chain.rpc_stats(),
            &function_metrics,
            "finished",
        );
        for stat in stats_ctx.function_stats(&function_metrics) {
            stat.log();
        }
        let assertion_word = if failed_assertions.len() == 1 {
            "assertion"
        } else {
            "assertions"
        };
        info!(
            "found {} distinct failed {assertion_word}",
            failed_assertions.len()
        );
        drop(_guard);
        drop(span);

        let shrink_span = info_span!("shrink", threads = shrink_threads);
        let _shrink_guard = shrink_span.enter();

        let runs_per_assertion = (session.args.shrink_runs / failed_assertions.len() as u64).max(1);
        let mut shrunk_assertions = Vec::with_capacity(failed_assertions.len());

        for (assertion_index, assertion) in failed_assertions.iter().enumerate() {
            let assertion_number = assertion_index + 1;
            let initial_calls = assertion.item.calls.len();

            // Combine the failing item with invariants so the shrinker operates
            // on a single corpus item and never appends invariants.
            // checkrs: allow(clone_in_loops)
            let mut combined_calls = assertion.item.calls.clone();
            // checkrs: allow(clone_in_loops)
            combined_calls.extend(invariant_calls.clone());
            let combined_item = Item::from(combined_calls);
            let shared_failed_item =
                // checkrs: allow(clone_in_loops)
                SharedFailedCorpusItem::new(combined_item, failed_corpus_config.clone());

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
                let shrinker_config = InvariantShrinkerConfig::new()
                    .chain(shrinker_chain)
                    .target_address(session.deployed_address)
                    .shared_failed_item(shrinker_shared_item)
                    .shutdown_signal(shrinker_shutdown)
                    .max_runs(local_max_runs)
                    .timeout(shrink_timeout)
                    .seed(seed)
                    // checkrs: allow(clone_in_loops)
                    .shared_metrics(shrinker_metrics.clone());
                let shrinker = InvariantShrinker::new(shrinker_config);
                let handle = std::thread::spawn(move || shrinker.run());
                shrinker_handles.push(handle);
            }

            info!(
                assertion = %format!("{assertion_number}/{}", failed_assertions.len()),
                initial_calls = %formatter::num(initial_calls as u64),
                "shrinking assertion",
            );
            wait_for_workers(&shrinker_handles, || {
                if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                    let current_calls = shared_failed_item.item().calls.len();
                    formatter::log_shrinker_progress(&snapshot, initial_calls, current_calls);
                }
                Ok(())
            })?;

            let mut failures: Vec<anyhow::Error> = Vec::new();
            for handle in shrinker_handles {
                match handle.join() {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        error!("shrinker failed: {e:#}");
                        failures.push(e);
                    }
                    Err(e) => {
                        error!(?e, "shrinker panicked");
                        failures.push(anyhow::anyhow!("shrinker panicked: {e:?}"));
                    }
                }
            }
            if !failures.is_empty() {
                let count = failures.len();
                let first = failures.remove(0);
                return Err(first).with_context(|| format!("{count} shrinker threads failed"));
            }

            let shrunk_item = shared_failed_item.item();
            let shrunk_calls = shrunk_item.calls.len();
            info!(
                assertion = %format!("{assertion_number}/{}", failed_assertions.len()),
                initial_calls = %formatter::num(initial_calls as u64),
                final_calls = %formatter::num(shrunk_calls as u64),
                "shrank assertion",
            );
            let summary = formatter::shrinker_summary(
                &shrinker_metrics.aggregate(),
                initial_calls,
                shrunk_calls,
            );
            info!(
                runs = %summary.runs,
                calls = %summary.calls,
                elapsed = %summary.elapsed,
                call_rate = %summary.call_rate,
                gas_rate = %summary.gas_rate,
                initial_calls = %summary.initial_calls,
                final_calls = %summary.final_calls,
                "shrinker statistics",
            );
            // Persist the shrunk sequence so the next campaign's corpus replay
            // discovers the failure directly from the shortest calls.
            // checkrs: allow(clone_in_loops)
            if let Err(e) = session.corpus.add_item(shrunk_item.clone()) {
                warn!("failed to persist shrunk sequence: {e:#}");
            }
            shrunk_assertions.push((assertion_number, shrunk_item));
        }
        drop(_shrink_guard);

        let trace_span = info_span!("trace");
        let _trace_guard = trace_span.enter();

        // Re-run each shrunk item with the chain tracer enabled, dumping the
        // decoded logs into the log and the full trace to a trace file.
        for (assertion_number, shrunk_item) in &shrunk_assertions {
            let transactions: Vec<Transaction> =
                shrunk_item
                    .calls
                    .iter()
                    .map(|call| call.into_transaction(session.deployed_address))
                    .chain(session.harness_contract.summary_transaction(
                        session.deployed_address,
                        session.args.deployer_address,
                    ))
                    .collect();

            let trace_name = if failed_assertions.len() == 1 {
                "fulltrace.log".to_owned()
            } else {
                format!("fulltrace-{assertion_number}.log")
            };
            match session.trace_sequence_to_file(&transactions, &trace_name) {
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

        if let Some(log_file) = &session.log_file {
            let log_span = info_span!("log");
            let _log_guard = log_span.enter();
            info!("{}", log_file.display());
        }

        if let Err(e) = session.write_coverage_report() {
            error!("failed to generate coverage reports: {e:#}");
        }

        Ok(())
    }
}
