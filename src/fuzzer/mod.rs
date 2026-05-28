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
//! * [`Fuzzer`](fuzzer::Fuzzer) is configured via [`Config`](config::Config)
//!   and runs directly on a cloned chain.

pub use config::Config;
pub use format::format_failure;
pub use fuzzer::{FailedAssertion, Fuzzer, RunOutput};
pub use metrics::{SharedMetrics, Snapshot};

pub use corpus::replayer::CorpusReplayer;

pub mod config;
pub mod corpus;
pub mod format;
#[allow(clippy::module_inception)]
pub mod fuzzer;
pub mod metrics;

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use crate::evm::Contract;
    use crate::evm::chain::Transaction;
    use crate::foundry;
    use crate::fuzzer::{FailedAssertion, format_failure};

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let project = foundry::Project::new("fixtures/challenges");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from("src/L1SimpleKnob.sol:SimpleKnob").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

        let transactions = vec![
            Transaction::new(Address::ZERO),
            Transaction::new(Address::ZERO),
            Transaction::new(Address::ZERO),
        ];

        let failure = FailedAssertion { transactions };

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
