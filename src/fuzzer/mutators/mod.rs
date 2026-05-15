pub mod abi;
pub mod corpus;
pub mod sequence;

pub use abi::SequenceArgMutator;
pub use corpus::{
    SequenceHeadMutator, SequenceInterleaveMutator, SequenceSpliceMutator, SequenceTailMutator,
};
pub use sequence::{
    SequenceDelayMutator, SequenceDeleteMutator, SequenceInsertMutator, SequenceSwapMutator,
};

#[cfg(test)]
mod tests {
    use libafl::mutators::Mutator;
    use libafl::state::HasRand;
    use libafl_bolts::rands::StdRand;

    use crate::fuzzer::mutators::{SequenceDelayMutator, SequenceInsertMutator};
    use crate::fuzzer::sequence::{Call, CallSequenceInput};

    /// Minimal test state that only implements `HasRand` so mutators can be
    /// exercised deterministically.
    struct MockState {
        rand: StdRand,
    }

    impl MockState {
        fn with_seed(seed: u64) -> Self {
            Self {
                rand: StdRand::with_seed(seed),
            }
        }
    }

    impl HasRand for MockState {
        type Rand = StdRand;
        fn rand(&self) -> &Self::Rand {
            &self.rand
        }
        fn rand_mut(&mut self) -> &mut Self::Rand {
            &mut self.rand
        }
    }

    #[test]
    fn sequence_delay_mutator_respects_cap_invariant() {
        let mut state = MockState::with_seed(42);
        let mut mutator = SequenceDelayMutator::new(10, 10);

        // Start with a single call whose delays are deliberately out of bounds.
        let mut input = CallSequenceInput {
            calls: vec![Call {
                selector: [0x12, 0x34, 0x56, 0x78],
                args: vec![0u8; 32],
                block_number_delay: 99,
                block_timestamp_delay: 1,
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        let call = &input.calls[0];
        assert!(
            call.block_number_delay <= call.block_timestamp_delay,
            "block_number_delay ({}) should be <= block_timestamp_delay ({}) after cap_delays",
            call.block_number_delay,
            call.block_timestamp_delay
        );
    }

    #[test]
    fn sequence_insert_mutator_respects_cap_invariant() {
        let mut state = MockState::with_seed(42);
        let selectors: Vec<[u8; 4]> = vec![[0x12, 0x34, 0x56, 0x78]];
        let mut mutator = SequenceInsertMutator::new(
            selectors, /* max_block_delay */ 10, /* max_time_delay */ 10,
        );

        let mut input = CallSequenceInput::new();
        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);
        assert_eq!(input.calls.len(), 1);

        let call = &input.calls[0];
        assert!(
            call.block_number_delay <= call.block_timestamp_delay,
            "inserted call should have capped delays: {} <= {}",
            call.block_number_delay,
            call.block_timestamp_delay
        );
    }

    #[test]
    fn mutated_sequence_advances_blocks_when_zero_delay_follows_nonzero() {
        use std::path::Path;

        use crate::contract::ContractBuilder;
        use crate::evm::EvmRunner;

        let artifact = ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        let runner = EvmRunner::from_target(&artifact).unwrap();

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

        // Build a 3-call seed sequence.
        let mut input = CallSequenceInput {
            calls: vec![
                Call {
                    selector: one,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                },
                Call {
                    selector: two,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                },
                Call {
                    selector: three,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                },
            ],
        };

        // Use the delay mutator to perturb delays.  With a fixed seed we get
        // a deterministic sequence, and because max delays are large enough the
        // mutator will almost certainly produce at least one non-zero delay.
        let mut state = MockState::with_seed(12345);
        let mut mutator = SequenceDelayMutator::new(5, 5);
        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        // Verify every call obeys the cap invariant.
        for (i, call) in input.calls.iter().enumerate() {
            assert!(
                call.block_number_delay <= call.block_timestamp_delay,
                "call {}: {} <= {}",
                i,
                call.block_number_delay,
                call.block_timestamp_delay
            );
        }

        // Run the mutated sequence and inspect block progression.
        let res = runner.run_sequence(&input.calls, crate::inspector::CoverageInspector::global()).unwrap();
        assert!(res.all_ok, "sequence should succeed");
        assert!(res.property_triggered, "property should be triggered");

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
