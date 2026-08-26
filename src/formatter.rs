//! Human-readable formatting for fuzzing campaign statistics.

use std::time::Duration;

use alloy_primitives::U256;
use alloy_primitives::utils::format_ether;
use tracing::info;

use crate::corpus::SharedCorpus;
use crate::evm::{RpcStats, SharedCoverage};
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

    /// Log a maxxing snapshot with the current best max value.
    pub fn log_maxxing_summary(
        &self,
        snapshot: &Snapshot,
        max_value: U256,
        rpc: RpcStats,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
        message: &str,
    ) {
        let summary = self.summary(snapshot);
        let hot = self.hotspot_fields(function_metrics);
        info!(
            runs = %summary.runs,
            calls = %summary.calls,
            elapsed = %summary.elapsed,
            call_rate = %summary.call_rate,
            gas_rate = %summary.gas_rate,
            rpc_hit = %num(rpc.hits),
            rpc_miss = %num(rpc.misses),
            rpc_wait = %duration(rpc.wait.as_secs_f64()),
            hot = %hot.function,
            hot_elapsed = %hot.elapsed,
            hot_rpc_miss = %hot.rpc_miss,
            value = %max_value,
            contracts = %summary.contracts,
            coverage = %summary.coverage,
            corpus = %summary.corpus,
            "{message}",
        );
    }

    /// Log a campaign statistics snapshot as structured `key=value` fields.
    ///
    /// Shared by the periodic progress updates and the final summary, so every
    /// campaign line parses with the same field names.
    pub fn log_summary(
        &self,
        snapshot: &Snapshot,
        rpc: RpcStats,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
        message: &str,
    ) {
        let summary = self.summary(snapshot);
        let hot = self.hotspot_fields(function_metrics);
        info!(
            runs = %summary.runs,
            calls = %summary.calls,
            elapsed = %summary.elapsed,
            call_rate = %summary.call_rate,
            gas_rate = %summary.gas_rate,
            rpc_hit = %num(rpc.hits),
            rpc_miss = %num(rpc.misses),
            rpc_wait = %duration(rpc.wait.as_secs_f64()),
            hot = %hot.function,
            hot_elapsed = %hot.elapsed,
            hot_rpc_miss = %hot.rpc_miss,
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

    /// The function that has spent the most wall time, if any is non-zero.
    pub fn hotspot(
        &self,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
    ) -> Option<Hotspot> {
        let mut best: Option<Hotspot> = None;
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
                if metrics.elapsed.is_zero() && metrics.rpc.misses == 0 {
                    continue;
                }
                let better = match &best {
                    None => true,
                    Some(current) => {
                        metrics.elapsed > current.elapsed
                            || (metrics.elapsed == current.elapsed
                                && metrics.rpc.misses > current.rpc_miss)
                    }
                };
                if better {
                    best = Some(Hotspot {
                        kind,
                        // checkrs: allow(clone_in_loops)
                        function: func.name.clone(),
                        elapsed: metrics.elapsed,
                        rpc_miss: metrics.rpc.misses,
                    });
                }
            }
        }
        best
    }

    fn hotspot_fields(
        &self,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
    ) -> HotspotFields {
        match self.hotspot(function_metrics) {
            Some(hot) => HotspotFields {
                function: hot.function,
                elapsed: duration(hot.elapsed.as_secs_f64()),
                rpc_miss: num(hot.rpc_miss),
            },
            None => HotspotFields {
                function: "-".into(),
                elapsed: duration(0.0),
                rpc_miss: num(0),
            },
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
                    elapsed: duration(metrics.elapsed.as_secs_f64()),
                    rpc_hit: num(metrics.rpc.hits),
                    rpc_miss: num(metrics.rpc.misses),
                    rpc_wait: duration(metrics.rpc.wait.as_secs_f64()),
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

/// The current slowest function by wall time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    pub kind: &'static str,
    pub function: String,
    pub elapsed: Duration,
    pub rpc_miss: u64,
}

struct HotspotFields {
    function: String,
    elapsed: String,
    rpc_miss: String,
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
    pub elapsed: String,
    pub rpc_hit: String,
    pub rpc_miss: String,
    pub rpc_wait: String,
    pub reverts: String,
}

impl FunctionStat {
    /// Log this row as structured fields.
    pub fn log(&self) {
        info!(
            calls = %self.calls,
            gas = %self.gas,
            elapsed = %self.elapsed,
            rpc_hit = %self.rpc_hit,
            rpc_miss = %self.rpc_miss,
            rpc_wait = %self.rpc_wait,
            reverts = %self.reverts,
            "{} {}",
            self.kind,
            self.function,
        );
    }
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
    use crate::evm::{RpcStats, SharedCoverage};

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
        assert_eq!(rows[0].elapsed, "0.00s");
        assert_eq!(rows[0].rpc_hit, "0");
        assert_eq!(rows[0].rpc_miss, "0");
        assert_eq!(rows[0].rpc_wait, "0.00s");
        assert_eq!(rows[0].reverts, "0");
    }

    #[test]
    fn hotspot_picks_max_elapsed_and_skips_cold_handlers() {
        let coverage = SharedCoverage::new();
        let corpus = SharedCorpus::new(CorpusConfig::new(""));
        let handlers = [
            alloy_json_abi::Function::parse("getQuote(uint24)").unwrap(),
            alloy_json_abi::Function::parse("swap()").unwrap(),
        ];
        let stats = CampaignStats::new(&coverage, &corpus, &handlers, &[], &[]);

        let cold = FunctionMetricsSnapshot {
            calls: 10,
            gas: 1_000,
            ..Default::default()
        };
        let hot = FunctionMetricsSnapshot {
            calls: 2,
            elapsed: Duration::from_millis(11_800),
            rpc: RpcStats {
                hits: 4,
                misses: 48,
                wait: Duration::from_millis(11_200),
            },
            ..Default::default()
        };
        let metrics = vec![
            (handlers[1].signature(), cold),
            (handlers[0].signature(), hot),
        ];

        let hotspot = stats.hotspot(&metrics).expect("expected a hotspot");
        assert_eq!(hotspot.kind, "handler");
        assert_eq!(hotspot.function, "getQuote");
        assert_eq!(hotspot.elapsed, Duration::from_millis(11_800));
        assert_eq!(hotspot.rpc_miss, 48);

        let none = stats.hotspot(&[(handlers[1].signature(), cold)]);
        assert_eq!(none, None);
    }
}
