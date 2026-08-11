//! Human-readable formatting for fuzzing campaign statistics.

use alloy_primitives::U256;
use alloy_primitives::utils::format_ether;

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

    /// Format a one-line fuzzing progress update.
    ///
    /// Keeps the campaign-level numbers that matter while fuzzing: runs,
    /// calls, elapsed time, throughput, coverage, and corpus size.
    pub fn progress(&self, snapshot: &Snapshot) -> String {
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

        format!(
            "fuzzing · {} runs · {} calls · {} · {} c/s · {} · cov {}e/{}d/{}r/{}j · {} corpus",
            num(snapshot.runs),
            num(snapshot.calls),
            duration(elapsed_secs),
            num(calls_per_sec),
            giga_gas(gas_per_sec) + "/s",
            num(self.shared_coverage.edge_count() as u64),
            num(self.shared_coverage.depth_count() as u64),
            num(self.shared_coverage.revert_count() as u64),
            num(self.shared_coverage.jump_count() as u64),
            num(self.corpus.stats().item_count as u64),
        )
    }

    /// Format the multi-line fuzzing statistics block.
    pub fn format(
        &self,
        snapshot: &Snapshot,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
    ) -> String {
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

        let mut output = format!(
            "\n    ⊕ global stats\n    total runs   : {}\n    total calls  : {}\n    elapsed time : {}\n\n    ⊕ throughput\n    call/s : {}\n    gas/s  : {}",
            num(snapshot.runs),
            num(snapshot.calls),
            duration(elapsed_secs),
            num(calls_per_sec),
            giga_gas(gas_per_sec),
        );

        output.push_str(&format!(
            "\n\n    ⊕ coverage stats\n    unique contracts : {}\n    total edges      : {}\n    total depths     : {}\n    total reverts    : {}\n    total jumps      : {}\n    total corpus     : {}",
            num(self.shared_coverage.contract_count() as u64),
            num(self.shared_coverage.edge_count() as u64),
            num(self.shared_coverage.depth_count() as u64),
            num(self.shared_coverage.revert_count() as u64),
            num(self.shared_coverage.jump_count() as u64),
            num(self.corpus.stats().item_count as u64),
        ));

        if !self.handler_functions.is_empty() {
            output.push_str(&format!(
                "\n\n    ⊕ handler functions ({})",
                self.handler_functions.len()
            ));
            let target_labels: Vec<String> = self
                .handler_functions
                .iter()
                .map(|f| f.name.to_string())
                .collect();
            let target_width = target_labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (func, label) in self.handler_functions.iter().zip(target_labels.iter()) {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                output.push_str(&format!(
                    "\n    {:target_width$} : {:>8} calls {:>10} gas {:>8} reverts",
                    label,
                    kmb(metrics.calls),
                    giga_gas(metrics.gas),
                    kmb(metrics.reverts),
                ));
            }
        }

        if !self.invariant_functions.is_empty() {
            output.push_str(&format!(
                "\n\n    ⊕ invariants ({})",
                self.invariant_functions.len()
            ));
            let invariant_labels: Vec<String> = self
                .invariant_functions
                .iter()
                .map(|f| f.name.to_string())
                .collect();
            let invariant_width = invariant_labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (func, label) in self.invariant_functions.iter().zip(invariant_labels.iter()) {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                output.push_str(&format!(
                    "\n    {:invariant_width$} : {:>8} calls {:>10} gas {:>8} reverts",
                    label,
                    kmb(metrics.calls),
                    giga_gas(metrics.gas),
                    kmb(metrics.reverts),
                ));
            }
        }

        if !self.max_functions.is_empty() {
            output.push_str(&format!(
                "\n\n    ⊕ max functions ({})",
                self.max_functions.len()
            ));
            let max_labels: Vec<String> = self
                .max_functions
                .iter()
                .map(|f| f.name.to_string())
                .collect();
            let max_width = max_labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (func, label) in self.max_functions.iter().zip(max_labels.iter()) {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                output.push_str(&format!(
                    "\n    {:max_width$} : {:>8} calls {:>10} gas {:>8} reverts",
                    label,
                    kmb(metrics.calls),
                    giga_gas(metrics.gas),
                    kmb(metrics.reverts),
                ));
            }
        }

        output
    }
}

/// Format a one-line shrinker progress update.
///
/// Keeps the campaign-level numbers plus the current size of the smallest
/// failing sequence so researchers can see shrinking progress.
pub fn shrinker_progress(
    snapshot: &Snapshot,
    initial_calls: usize,
    current_calls: usize,
) -> String {
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

    format!(
        "shrinking · {} runs · {} calls · {} · {} c/s · {} · {} → {} calls",
        num(snapshot.runs),
        num(snapshot.calls),
        duration(elapsed_secs),
        num(calls_per_sec),
        giga_gas(gas_per_sec) + "/s",
        num(initial_calls as u64),
        num(current_calls as u64),
    )
}

/// Format the multi-line shrinker statistics block shown after shrinking.
pub fn shrinker_summary(snapshot: &Snapshot, initial_calls: usize, current_calls: usize) -> String {
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

    format!(
        "\n    ⊕ shrinker stats\n    total runs   : {}\n    total calls  : {}\n    elapsed time : {}\n\n    ⊕ throughput\n    call/s : {}\n    gas/s  : {}\n\n    ⊕ shrink progress\n    initial calls : {}\n    final calls   : {}",
        num(snapshot.runs),
        num(snapshot.calls),
        duration(elapsed_secs),
        num(calls_per_sec),
        giga_gas(gas_per_sec),
        num(initial_calls as u64),
        num(current_calls as u64),
    )
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
    fn campaign_progress_preserves_key_stats() {
        let coverage = SharedCoverage::new();
        let corpus = SharedCorpus::new(CorpusConfig::new(""));
        let stats = CampaignStats::new(&coverage, &corpus, &[], &[], &[]);

        let line = stats.progress(&snapshot());

        assert!(line.contains("1,234 runs"), "{line}");
        assert!(line.contains("56,789 calls"), "{line}");
        assert!(line.contains("2.00s"), "{line}");
        assert!(line.contains("28,394 c/s"), "{line}");
        assert!(line.contains("1.00 G/s"), "{line}");
        assert!(line.contains("cov 0e/0d/0r/0j"), "{line}");
        assert!(line.contains("0 corpus"), "{line}");
    }

    #[test]
    fn shrinker_progress_shows_current_size() {
        let line = shrinker_progress(&snapshot(), 36, 3);

        assert!(line.contains("1,234 runs"), "{line}");
        assert!(line.contains("56,789 calls"), "{line}");
        assert!(line.contains("2.00s"), "{line}");
        assert!(line.contains("28,394 c/s"), "{line}");
        assert!(line.contains("1.00 G/s"), "{line}");
        assert!(line.contains("36 → 3 calls"), "{line}");
    }

    #[test]
    fn shrinker_summary_includes_initial_and_final() {
        let summary = shrinker_summary(&snapshot(), 36, 3);

        assert!(summary.contains("1,234"), "{summary}");
        assert!(summary.contains("56,789"), "{summary}");
        assert!(summary.contains("2.00s"), "{summary}");
        assert!(summary.contains("28,394"), "{summary}");
        assert!(summary.contains("1.00 G"), "{summary}");
        assert!(summary.contains("initial calls : 36"), "{summary}");
        assert!(summary.contains("final calls   : 3"), "{summary}");
    }
}
