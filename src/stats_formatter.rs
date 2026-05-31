//! Human-readable formatting for fuzzing campaign statistics.

use alloy_primitives::U256;
use alloy_primitives::utils::format_ether;

use crate::corpus::SharedCorpus;
use crate::evm::SharedCoverage;
use crate::fuzzer::{FunctionMetricsSnapshot, Snapshot};

/// Formatter for campaign statistics and related numeric values.
pub struct StatsFormatter;

impl StatsFormatter {
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

    /// Format the multi-line fuzzing statistics block.
    pub fn fuzzing_stats(
        snapshot: &Snapshot,
        function_metrics: &[(String, FunctionMetricsSnapshot)],
        shared_coverage: &SharedCoverage,
        corpus: &SharedCorpus,
        target_functions: &[alloy_json_abi::Function],
        invariant_functions: &[alloy_json_abi::Function],
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
            "\n    ⊕ global stats\n    total runs   : {}\n    total calls  : {}\n    elapsed time : {:.2}s\n\n    ⊕ throughput\n    call/s : {}\n    gas/s  : {}",
            Self::num(snapshot.runs),
            Self::num(snapshot.calls),
            elapsed_secs,
            Self::num(calls_per_sec),
            Self::giga_gas(gas_per_sec),
        );

        output.push_str(&format!(
            "\n\n    ⊕ coverage stats\n    unique contracts : {}\n    total edges      : {}\n    total depths     : {}\n    total reverts    : {}\n    total jumps      : {}\n    total corpus     : {}",
            Self::num(shared_coverage.contract_count() as u64),
            Self::num(shared_coverage.edge_count() as u64),
            Self::num(shared_coverage.depth_count() as u64),
            Self::num(shared_coverage.revert_count() as u64),
            Self::num(shared_coverage.jump_count() as u64),
            Self::num(corpus.stats().item_count as u64),
        ));

        if !target_functions.is_empty() {
            output.push_str(&format!(
                "\n\n    ⊕ target functions ({})",
                target_functions.len()
            ));
            let target_labels: Vec<String> = target_functions
                .iter()
                .map(|f| format!("{} ({})", f.name, f.selector()))
                .collect();
            let target_width = target_labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (func, label) in target_functions.iter().zip(target_labels.iter()) {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                output.push_str(&format!(
                    "\n    {:target_width$} : {:>8} calls {:>10} gas {:>8} reverts",
                    label,
                    Self::kmb(metrics.calls),
                    Self::giga_gas(metrics.gas),
                    Self::kmb(metrics.reverts),
                ));
            }
        }

        if !invariant_functions.is_empty() {
            output.push_str(&format!(
                "\n\n    ⊕ invariants ({})",
                invariant_functions.len()
            ));
            let invariant_labels: Vec<String> = invariant_functions
                .iter()
                .map(|f| format!("{} ({})", f.name, f.selector()))
                .collect();
            let invariant_width = invariant_labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (func, label) in invariant_functions.iter().zip(invariant_labels.iter()) {
                let sig = func.signature();
                let metrics = function_metrics
                    .iter()
                    .find(|(s, _)| s == &sig)
                    .map(|(_, m)| *m)
                    .unwrap_or_default();
                output.push_str(&format!(
                    "\n    {:invariant_width$} : {:>8} calls {:>10} gas {:>8} reverts",
                    label,
                    Self::kmb(metrics.calls),
                    Self::giga_gas(metrics.gas),
                    Self::kmb(metrics.reverts),
                ));
            }
        }

        output
    }
}
