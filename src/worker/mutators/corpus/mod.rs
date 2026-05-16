//! Mutators that splice, interleave, or trim corpus entries.

pub use head::SequenceHeadMutator;
pub use interleave::SequenceInterleaveMutator;
pub use splice::SequenceSpliceMutator;
pub use tail::SequenceTailMutator;
mod head;
mod interleave;
mod splice;
mod tail;
