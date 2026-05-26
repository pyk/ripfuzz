//! Coverage-guided, mutational stateful fuzzer.
//!
//! `crate::fuzzer` owns the EVM chain and orchestrates parallel fuzzing
//! threads via [`Factory`](factory::Factory) and [`Fuzzer`](factory::Fuzzer).

pub use config::Config;
pub use corpus::SharedCorpus;
pub use engine::{CrashInfo, ExecutionOutcome, is_assert_failure};
pub use factory::{Crash, Factory, Fuzzer, FuzzerResult, format_failure};
pub use metrics::{MetricsSnapshot, SharedMetrics};

pub mod config;
pub mod corpus;
pub mod engine;
pub mod factory;
pub mod metrics;
pub mod mutators;

#[cfg(test)]
mod tests {
    use crate::foundry;
    use crate::fuzzer::corpus::{Call, CallMeta};
    use crate::fuzzer::{Crash, format_failure};
    use crate::target::Contract;

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let project = foundry::Project::new("fixtures/challenges");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from("src/L1SimpleKnob.sol:SimpleKnob").unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        let contract = Contract::try_from(artifact).unwrap();

        let calls = vec![
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
                ..Default::default()
            },
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let failure = Crash {
            function_name: "invariant_caught".into(),
            selector: [0; 4],
            call_sequence: calls,
            call_meta: vec![
                CallMeta {
                    block_number: 0,
                    block_timestamp: 0,
                    ..Default::default()
                },
                CallMeta {
                    block_number: 3,
                    block_timestamp: 4,
                    ..Default::default()
                },
                CallMeta {
                    block_number: 4,
                    block_timestamp: 5,
                    ..Default::default()
                },
            ],
        };

        let output = format_failure(&contract, &failure, crate::evm::chain::DEFAULT_DEPLOYER);
        assert!(
            output.contains("block_number="),
            "output should use block_number label:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp="),
            "output should use block_timestamp label:\n{}",
            output
        );
        assert!(
            !output.contains("block=0") && !output.contains("block=3"),
            "output should not use old block= label:\n{}",
            output
        );
        assert!(
            !output.contains("time=1") && !output.contains("time=5"),
            "output should not use old time= label:\n{}",
            output
        );
        assert!(
            output.contains("block_number_delay=3"),
            "output should show block_number_delay:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp_delay=4"),
            "output should show block_timestamp_delay:\n{}",
            output
        );
    }
}
