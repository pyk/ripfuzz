//! Human-readable formatting for fuzzing campaign statistics.

use alloy_primitives::U256;
use alloy_primitives::utils::format_ether;
use tracing::info;

use crate::corpus::SharedCorpus;
use crate::evm::SharedCoverage;
use crate::fuzzers::{FunctionMetricsSnapshot, Snapshot};

/// Format a number with comma-separated thousands.
pub fn num(n: u64) -> String {
    let s = format!("{n}");
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (count, c) in s.chars().rev().enumerate() {
        if count > 0 && count % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Format a byte size as a human-readable KB string.
pub fn kb(size: usize) -> String {
    let kb = size as f64 / 1024.0;
    format!("{kb:.1} KB")
}

/// Format a wei value as a human-readable ETH string, trimming trailing zeros.
pub fn eth(value: U256) -> String {
    if value == U256::ZERO {
        return "0 ETH".into();
    }
    let s = format_ether(value);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed} ETH")
}

/// Format a call count using K/M/B suffixes.
pub fn kmb(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else if n < 1_000_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    }
}

/// Format a gas value as giga-gas.
pub fn giga_gas(n: u64) -> String {
    format!("{:.2} G", n as f64 / 1_000_000_000.0)
}

/// Format coverage counters as a compact breakdown, e.g. `1,234e 56d 7r 89j`.
pub fn coverage(edges: usize, depths: usize, reverts: usize, jumps: usize) -> String {
    format!(
        "{}e {}d {}r {}j",
        num(edges as u64),
        num(depths as u64),
        num(reverts as u64),
        num(jumps as u64)
    )
}

/// Format a duration (in seconds) as a human-readable string.
///
/// - Less than 1 minute: `{secs:.2}s` (e.g. `30.50s`)
/// - 1 minute or more:  `{xm}{ys}` (e.g. `8m30s`)
/// - 1 hour or more:    `{xh}{xm}{ys}` (e.g. `1h5m30s`)
pub fn duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.2}s")
    } else {
        let total_secs = secs as u64;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs_remainder = total_secs % 60;
        if hours > 0 {
            format!("{hours}h{mins}m{secs_remainder}s")
        } else {
            format!("{mins}m{secs_remainder}s")
        }
    }
}

/// Context for formatting fuzzing campaign statistics.
pub struct CampaignStats<'a> {
    shared_coverage: &'a SharedCoverage,
    corpus: &'a SharedCorpus,
    handler_functions: &'a [alloy_json_abi::Function],
    invariant_functions: &'a [alloy_json_abi::Function],
    max_functions: &'a [alloy_json_abi::Function],
}

impl<'a> CampaignStats<'a> {
    /// Create a new [`CampaignStats`] context.
    pub fn new(
        shared_coverage: &'a SharedCoverage,
        corpus: &'a SharedCorpus,
        handler_functions: &'a [alloy_json_abi::Function],
        invariant_functions: &'a [alloy_json_abi::Function],
        max_functions: &'a [alloy_json_abi::Function],
    ) -> Self {
        Self {
            shared_coverage,
            corpus,
            handler_functions,
            invariant_functions,
            max_functions,
        }
    }

    /// Log a campaign statistics snapshot as structured `key=value` fields.
    ///
    /// Shared by the periodic progress updates and the final summary, so every
    /// campaign line parses with the same field names.
    pub fn log_summary(&self, snapshot: &Snapshot, message: &str) {
        let summary = self.summary(snapshot);
        info!(
            runs = %summary.runs,
            calls = %summary.calls,
            elapsed = %summary.elapsed,
            call_rate = %summary.call_rate,
            gas_rate = %summary.gas_rate,
            contracts = %summary.contracts,
            coverage = %summary.coverage,
            corpus = %summary.corpus,
            "{message}",
        );
    }

    /// Aggregate the campaign-wide statistics for structured logging.
    pub fn summary(&self, snapshot: &Snapshot) -> CampaignSummary {
        let elapsed_secs = snapshot.elapsed.as_secs_f64();
        let calls_per_sec = if elapsed_secs > 0.0 {
            (snapshot.calls as f64 / elapsed_secs) as u64
        } else {
            0
        };
        let gas_per_sec = if elapsed_secs > 0.0 {
            (snapshot.gas as f64 / elapsed_secs) as u64
        } else {
            0
        };

        CampaignSummary {
            runs: num(snapshot.runs),
            calls: num(snapshot.calls),
            elapsed: duration(elapsed_secs),
            call_rate: num(calls_per_sec),
            gas_rate: giga_gas(gas_per_sec),
            contracts: num(self.shared_coverage.contract_count() as u64),
            coverage: coverage(
                self.shared_coverage.edge_count(),
                self.shared_coverage.depth_count(),
                self.shared_coverage.revert_count(),
                self.shared_coverage.jump_count(),
            ),
            corpus: num(self.corpus.stats().item_count as u64),
        }
    }

    /// Per-function call statistics in declaration order, for structured
    /// logging.
    pub fn function_stats(
        &self,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
    ) -> Vec<FunctionStat> {
        let mut stats = Vec::new();
        for (kind, functions) in [
            ("handler", self.handler_functions),
            ("invariant", self.invariant_functions),
            ("max", self.max_functions),
        ] {
            for func in functions {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                stats.push(FunctionStat {
                    kind,
                    // checkrs: allow(clone_in_loops)
                    function: func.name.clone(),
                    calls: kmb(metrics.calls),
                    gas: giga_gas(metrics.gas),
                    reverts: kmb(metrics.reverts),
                });
            }
        }
        stats
    }
}

/// Campaign-wide fuzzing statistics, pre-formatted for structured logging.
#[derive(Debug)]
pub struct CampaignSummary {
    pub runs: String,
    pub calls: String,
    pub elapsed: String,
    pub call_rate: String,
    pub gas_rate: String,
    pub contracts: String,
    pub coverage: String,
    pub corpus: String,
}

/// One row of per-function call statistics, pre-formatted for structured
/// logging.
#[derive(Debug)]
pub struct FunctionStat {
    /// Function category: `handler`, `invariant`, or `max`.
    pub kind: &'static str,
    pub function: String,
    pub calls: String,
    pub gas: String,
    pub reverts: String,
}

/// Format a one-line shrinker progress update.
///
/// Keeps the campaign-level numbers plus the current size of the smallest
/// failing sequence so researchers can see shrinking progress.
/// Log a mid-shrink progress snapshot with structured fields, mirroring the
/// fuzz progress summary.
pub fn log_shrinker_progress(snapshot: &Snapshot, initial_calls: usize, current_calls: usize) {
    let elapsed_secs = snapshot.elapsed.as_secs_f64();
    let calls_per_sec = if elapsed_secs > 0.0 {
        (snapshot.calls as f64 / elapsed_secs) as u64
    } else {
        0
    };
    let gas_per_sec = if elapsed_secs > 0.0 {
        (snapshot.gas as f64 / elapsed_secs) as u64
    } else {
        0
    };

    info!(
        runs = %num(snapshot.runs),
        calls = %num(snapshot.calls),
        elapsed = %duration(elapsed_secs),
        call_rate = %num(calls_per_sec),
        gas_rate = %giga_gas(gas_per_sec),
        initial_calls = %num(initial_calls as u64),
        current_calls = %num(current_calls as u64),
        "progress",
    );
}

/// Shrinker statistics, pre-formatted for structured logging.
#[derive(Debug)]
pub struct ShrinkerSummary {
    pub runs: String,
    pub calls: String,
    pub elapsed: String,
    pub call_rate: String,
    pub gas_rate: String,
    pub initial_calls: String,
    pub final_calls: String,
}

/// Aggregate the shrinker statistics for structured logging.
pub fn shrinker_summary(
    snapshot: &Snapshot,
    initial_calls: usize,
    current_calls: usize,
) -> ShrinkerSummary {
    let elapsed_secs = snapshot.elapsed.as_secs_f64();
    let calls_per_sec = if elapsed_secs > 0.0 {
        (snapshot.calls as f64 / elapsed_secs) as u64
    } else {
        0
    };
    let gas_per_sec = if elapsed_secs > 0.0 {
        (snapshot.gas as f64 / elapsed_secs) as u64
    } else {
        0
    };

    ShrinkerSummary {
        runs: num(snapshot.runs),
        calls: num(snapshot.calls),
        elapsed: duration(elapsed_secs),
        call_rate: num(calls_per_sec),
        gas_rate: giga_gas(gas_per_sec),
        initial_calls: num(initial_calls as u64),
        final_calls: num(current_calls as u64),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::corpus::{CorpusConfig, SharedCorpus};
    use crate::evm::SharedCoverage;

    fn snapshot() -> Snapshot {
        Snapshot {
            elapsed: Duration::from_secs_f64(2.0),
            runs: 1_234,
            calls: 56_789,
            gas: 2_000_000_000,
        }
    }

    #[test]
    fn shrinker_summary_preserves_initial_and_final() {
        let summary = shrinker_summary(&snapshot(), 36, 3);

        assert_eq!(summary.runs, "1,234");
        assert_eq!(summary.calls, "56,789");
        assert_eq!(summary.elapsed, "2.00s");
        assert_eq!(summary.call_rate, "28,394");
        assert_eq!(summary.gas_rate, "1.00 G");
        assert_eq!(summary.initial_calls, "36");
        assert_eq!(summary.final_calls, "3");
    }

    #[test]
    fn campaign_summary_preserves_key_stats() {
        let coverage = SharedCoverage::new();
        let corpus = SharedCorpus::new(CorpusConfig::new(""));
        let stats = CampaignStats::new(&coverage, &corpus, &[], &[], &[]);

        let summary = stats.summary(&snapshot());

        assert_eq!(summary.runs, "1,234");
        assert_eq!(summary.calls, "56,789");
        assert_eq!(summary.elapsed, "2.00s");
        assert_eq!(summary.call_rate, "28,394");
        assert_eq!(summary.gas_rate, "1.00 G");
        assert_eq!(summary.contracts, "0");
        assert_eq!(summary.coverage, "0e 0d 0r 0j");
        assert_eq!(summary.corpus, "0");
    }

    #[test]
    fn function_stats_follow_declaration_order() {
        let coverage = SharedCoverage::new();
        let corpus = SharedCorpus::new(CorpusConfig::new(""));
        let handlers = [alloy_json_abi::Function::parse("f(uint256)").unwrap()];
        let invariants = [alloy_json_abi::Function::parse("invariant()").unwrap()];
        let stats = CampaignStats::new(&coverage, &corpus, &handlers, &invariants, &[]);

        let rows = stats.function_stats(&[]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "handler");
        assert_eq!(rows[0].function, "f");
        assert_eq!(rows[1].kind, "invariant");
        assert_eq!(rows[1].function, "invariant");
        assert_eq!(rows[0].calls, "0");
        assert_eq!(rows[0].gas, "0.00 G");
        assert_eq!(rows[0].reverts, "0");
    }
}
