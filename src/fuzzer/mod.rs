//! Coverage-guided, mutational stateful fuzzer.
//!
//! [`Fuzzer`](fuzzer::Fuzzer) owns the execution loop: calling
//! [`next_item`](crate::corpus::SharedCorpus::next_item) to obtain an input,
//! executing it against a cloned chain, and calling
//! [`add_item`](crate::corpus::SharedCorpus::add_item) to store interesting
//! sequences discovered during execution.
//!
//! [`Fuzzer`](fuzzer::Fuzzer) is configured via [`Config`](config::Config)
//! and runs directly on a cloned chain.

pub use config::Config;
pub use fuzzer::{FailedAssertion, Fuzzer, RunOutput};
pub use metrics::{FunctionMetricsSnapshot, SharedMetrics, Snapshot};

mod config;
#[allow(clippy::module_inception)]
mod fuzzer;
mod metrics;

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use crate::corpus::Item;
    use crate::evm::Contract;
    use crate::evm::Transaction;
    use crate::foundry;
    use crate::fuzzer::FailedAssertion;

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

        let failure = FailedAssertion {
            transactions,
            item: Item::from(vec![]),
        };

        let output = failure.format(&contract, crate::evm::DEFAULT_DEPLOYER);
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
