use std::hash::{Hash, Hasher};

use libafl::inputs::{HasTargetBytes, Input};
use libafl_bolts::ownedref::OwnedSlice;
use serde::{Deserialize, Serialize};

/// A single call in a sequence.
#[derive(Clone, Debug, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    /// 4-byte function selector.
    pub selector: [u8; 4],
    /// ABI-encoded arguments (0 or more 32-byte words).
    pub args: Vec<u8>,
    /// How many blocks to advance before this call is executed.
    pub block_number_delay: u64,
    /// How many seconds to advance before this call is executed.
    pub block_timestamp_delay: u64,
}

impl Call {
    /// Total encoded size of this call (selector + args).
    pub fn encoded_size(&self) -> usize {
        4 + self.args.len()
    }

    /// Encode this call as a flat byte vector.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_size());
        buf.extend_from_slice(&self.selector);
        buf.extend_from_slice(&self.args);
        buf
    }

    /// Cap block number delay so it never exceeds timestamp delay.
    /// Medusa invariant: each block must have a unique timestamp.
    pub fn cap_delays(&mut self) {
        if self.block_number_delay > self.block_timestamp_delay {
            if self.block_timestamp_delay == 0 {
                self.block_number_delay = 0;
            } else {
                self.block_number_delay %= self.block_timestamp_delay;
            }
        }
    }
}

/// A sequence of calls targeting a deployed contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallSequenceInput {
    pub calls: Vec<Call>,
}

impl CallSequenceInput {
    /// Create an empty sequence.
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }

    /// Create a sequence from a single call.
    pub fn single(call: Call) -> Self {
        Self { calls: vec![call] }
    }

    /// Flatten the entire sequence into a single byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.calls.iter().map(|c| c.encoded_size()).sum());
        for call in &self.calls {
            buf.extend_from_slice(&call.selector);
            buf.extend_from_slice(&call.args);
        }
        buf
    }
}

impl Default for CallSequenceInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Input for CallSequenceInput {}

impl HasTargetBytes for CallSequenceInput {
    fn target_bytes(&self) -> OwnedSlice<'_, u8> {
        OwnedSlice::from(self.to_bytes())
    }
}

// Manual Hash impl because we want to hash the flat bytes.
impl Hash for CallSequenceInput {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_bytes().hash(state);
    }
}

impl PartialEq for CallSequenceInput {
    fn eq(&self, other: &Self) -> bool {
        self.to_bytes() == other.to_bytes()
    }
}

impl Eq for CallSequenceInput {}
