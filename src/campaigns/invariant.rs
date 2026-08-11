//! Invariant testing campaign: validate invariants across generated call
//! sequences and shrink every distinct failed assertion.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, ensure};
use tracing::{error, info};

use crate::campaigns::{CampaignKind, CampaignSession, split_runs, wait_for_workers};
use crate::corpus::{Call, CorpusConfig, Item, SharedFailedCorpusItem};
use crate::evm::Transaction;
use crate::formatter;
use crate::fuzzer::{Fuzzer, FuzzerConfig, SharedFailedAssertions, SharedMetrics};
use crate::shrinker::{Shrinker, ShrinkerConfig};

/// Invariant testing campaign.
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
        let InvariantCampaign { session } = self;

        let all_function_signatures: Vec<String> = session
            .harness_contract
            .handler_functions
            .iter()
            .chain(session.harness_contract.invariant_functions.iter())
            .map(|f| f.signature())
            .collect();
        let shared_metrics = SharedMetrics::new(all_function_signatures.clone());
        let shared_failed_assertions = SharedFailedAssertions::for_campaign(
            session.args.max_failures,
            session.args.fail_on_revert,
        );
        let shutdown_signal = Arc::new(AtomicBool::new(false));

        let fuzzers = session.args.threads;
        let timeout = session
            .args
            .timeout_secs
            .map(std::time::Duration::from_secs);

        let initial_config = FuzzerConfig::new()
            .chain(session.chain.clone())
            .target_address(session.deployed_address)
            .shared_corpus(session.corpus.clone())
            .shared_coverage(session.shared_coverage.clone())
            .shared_metrics(shared_metrics.clone())
            .shared_failed_assertions(shared_failed_assertions.clone())
            .shutdown_signal(shutdown_signal.clone())
            .invariant_functions(session.harness_contract.invariant_functions.clone())
            .caller(session.args.deployer_address)
            .gas_limit(session.args.gas_limit)
            .timeout(timeout)
            .fail_on_revert(session.args.fail_on_revert);

        let mut handles = Vec::with_capacity(fuzzers);
        for (fuzzer_id, local_max_runs) in split_runs(session.args.max_runs, fuzzers).enumerate() {
            let seed = session.campaign_seed.wrapping_add(fuzzer_id as u64);
            // checkrs: allow(clone_in_loops)
            let mut config = initial_config.clone();
            config.max_runs = local_max_runs;
            config.seed = seed;

            let fuzzer = Fuzzer::new(config);
            let handle = std::thread::spawn(move || fuzzer.run());
            handles.push((fuzzer_id, handle));
        }

        let contract_name = session.contract_name();
        info!("[*] fuzzing {contract_name} with {fuzzers} threads");

        // Print a compact progress line every 3 seconds, then a full stats
        // summary after all fuzzer threads finish.
        let stats_ctx = formatter::CampaignStats::new(
            &session.shared_coverage,
            &session.corpus,
            &session.harness_contract.handler_functions,
            &session.harness_contract.invariant_functions,
            &[],
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
                    error!(fuzzer_id, %e, "fuzzer failed");
                }
                Err(e) => {
                    error!(fuzzer_id, ?e, "fuzzer panicked");
                }
            }
        }

        let failed_assertions = shared_failed_assertions.items();
        if failed_assertions.is_empty() {
            info!("[+] fuzzed {contract_name} with {fuzzers} threads");
            let function_metrics = shared_metrics.function_metrics();
            let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
            info!("{stats}");

            if let Err(e) = session.write_coverage_report() {
                error!("[!] failed to generate coverage reports: {e:#}");
            }

            info!("[*] no failed assertions found!");
            info!("[*] ripfuzz out. see ya");
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

        info!("[+] fuzzed {contract_name} with {fuzzers} threads");
        let function_metrics = shared_metrics.function_metrics();
        let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
        info!("{stats}");
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
                    .fail_on_revert(session.args.fail_on_revert);
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

        // Re-run each shrunk item with the chain tracer enabled.
        for (assertion_number, shrunk_item) in &shrunk_assertions {
            // checkrs: allow(clone_in_loops)
            let mut trace_chain = session.chain.clone();
            trace_chain.set_trace(true);

            let transactions: Vec<Transaction> = shrunk_item
                .calls
                .iter()
                .map(|call| call.into_transaction(session.deployed_address))
                .collect();

            let exec = trace_chain.exec(&transactions)?;

            // TODO(pyk): assert that trace should exists; do not use if else
            if let Some(trace) = exec.trace {
                let trace_name = if failed_assertions.len() == 1 {
                    "trace.log".to_owned()
                } else {
                    format!("trace-{assertion_number}.log")
                };
                info!("[*] writing trace {assertion_number} ...");
                match session.write_trace(&trace, &trace_name) {
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

        if let Err(e) = session.write_coverage_report() {
            error!("[!] failed to generate coverage reports: {e:#}");
        }

        info!("[*] ripfuzz out. see ya");
        Ok(())
    }
}
