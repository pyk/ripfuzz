//! Coverage-guided, mutational stateful fuzzer.
//!
//! ## Separation of concerns
//!
//! * [`SharedCorpus`](corpus::SharedCorpus) owns the corpus lifecycle:
//!   loading from disk, serialization, weighted random selection, mutation,
//!   and coverage-driven insertion.
//! * [`Fuzzer`](fuzzer::Fuzzer) owns the execution loop: calling
//!   [`next_item`](corpus::SharedCorpus::next_item) to obtain an input, executing it
//!   against a cloned chain, and calling [`add_item`](corpus::SharedCorpus::add_item)
//!   when the input is interesting.
//! * [`Factory`](factory::Factory) creates per-thread [`Fuzzer`] instances
//!   from the post-setup chain snapshot.

pub use config::Config;
pub use engine::{CrashInfo, ExecutionOutcome, is_assert_failure};
pub use factory::{Crash, Factory, FuzzerResult, format_failure};
pub use fuzzer::Fuzzer;
pub use metrics::{SharedMetrics, Snapshot};

pub mod config;
pub mod corpus;
pub mod engine;
pub mod factory;
#[allow(clippy::module_inception)]
pub mod fuzzer;
pub mod metrics;
pub mod mutators;

#[cfg(test)]
mod tests {
    use crate::evm::Contract;
    use crate::foundry;
    use crate::fuzzer::corpus::Call;
    use crate::fuzzer::{Crash, format_failure};

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let project = foundry::Project::new("fixtures/challenges");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from("src/L1SimpleKnob.sol:SimpleKnob").unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        let contract = Contract::try_from(artifact).unwrap();

        let func = alloy_json_abi::Function::parse("foo()").unwrap();
        let calls = vec![
            Call {
                function: func.clone(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![]),
                ..Default::default()
            },
            Call {
                function: func.clone(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![]),
                ..Default::default()
            },
            Call {
                function: func.clone(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![]),
                ..Default::default()
            },
        ];

        let failure = Crash {
            function_name: "invariant_caught".into(),
            selector: alloy_primitives::Selector::ZERO,
            call_sequence: calls,
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
            !output.contains("block=0") && !output.contains("block=1"),
            "output should not use old block= label:\n{}",
            output
        );
        assert!(
            !output.contains("time=1") && !output.contains("time=2"),
            "output should not use old time= label:\n{}",
            output
        );
    }
}
