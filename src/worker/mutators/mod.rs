//! Mutators that transform call sequences during fuzzing.

pub use abi::SequenceArgMutator;
pub use corpus::{
    SequenceHeadMutator, SequenceInterleaveMutator, SequenceSpliceMutator, SequenceTailMutator,
};
pub use sequence::{
    SequenceDelayMutator, SequenceDeleteMutator, SequenceInsertMutator, SequenceSwapMutator,
};

use crate::corpus::Call;

pub mod abi;
pub mod corpus;
pub mod sequence;

/// Result of applying a mutator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationResult {
    /// The input was changed.
    Mutated,
    /// The input was left unchanged.
    Skipped,
}

/// Trait for mutators that operate on a call sequence.
pub trait Mutator {
    /// Mutate `calls` in place.
    fn mutate(&mut self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult;
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus;
    use crate::worker::mutators;
    use crate::worker::mutators::Mutator;

    #[test]
    fn sequence_delay_mutator_respects_cap_invariant() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mut mutator = mutators::SequenceDelayMutator::new(10, 10);

        let mut calls = vec![corpus::Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            block_number_delay: 99,
            block_timestamp_delay: 1,
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, mutators::MutationResult::Mutated);

        let call = &calls[0];
        assert!(
            call.block_number_delay <= call.block_timestamp_delay,
            "block_number_delay ({}) should be <= block_timestamp_delay ({}) after cap_delays",
            call.block_number_delay,
            call.block_timestamp_delay
        );
    }

    #[test]
    fn sequence_insert_mutator_respects_cap_invariant() {
        let mut rng = fastrand::Rng::with_seed(42);
        let selectors: Vec<[u8; 4]> = vec![[0x12, 0x34, 0x56, 0x78]];
        let mut mutator = mutators::SequenceInsertMutator::new(selectors, 10, 10);

        let mut calls = Vec::new();
        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, mutators::MutationResult::Mutated);
        assert_eq!(calls.len(), 1);

        let call = &calls[0];
        assert!(
            call.block_number_delay <= call.block_timestamp_delay,
            "inserted call should have capped delays: {} <= {}",
            call.block_number_delay,
            call.block_timestamp_delay
        );
    }

    #[test]
    fn mutated_sequence_advances_blocks_when_zero_delay_follows_nonzero() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();

        let one = artifact
            .abi
            .functions()
            .find(|f| f.name == "one")
            .unwrap()
            .selector()
            .into();
        let two = artifact
            .abi
            .functions()
            .find(|f| f.name == "two")
            .unwrap()
            .selector()
            .into();
        let three = artifact
            .abi
            .functions()
            .find(|f| f.name == "three")
            .unwrap()
            .selector()
            .into();

        let mut calls = vec![
            corpus::Call {
                selector: one,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            corpus::Call {
                selector: two,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            corpus::Call {
                selector: three,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let mut rng = fastrand::Rng::with_seed(12345);
        let mut mutator = mutators::SequenceDelayMutator::new(5, 5);
        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, mutators::MutationResult::Mutated);

        for (i, call) in calls.iter().enumerate() {
            assert!(
                call.block_number_delay <= call.block_timestamp_delay,
                "call {}: {} <= {}",
                i,
                call.block_number_delay,
                call.block_timestamp_delay
            );
        }

        let res = chain.execute(&calls).unwrap();
        assert!(
            res.crash.is_some(),
            "invariant should be triggered (assert panic)"
        );

        for i in 1..res.call_meta.len() {
            assert!(
                res.call_meta[i].block_number > res.call_meta[i - 1].block_number,
                "call {} block ({}) should be > call {} block ({})",
                i,
                res.call_meta[i].block_number,
                i - 1,
                res.call_meta[i - 1].block_number
            );
            assert!(
                res.call_meta[i].block_timestamp >= res.call_meta[i - 1].block_timestamp,
                "call {} timestamp ({}) should be >= call {} timestamp ({})",
                i,
                res.call_meta[i].block_timestamp,
                i - 1,
                res.call_meta[i - 1].block_timestamp
            );
        }
    }
}
